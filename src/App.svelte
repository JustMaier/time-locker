<script>
  import { onMount, onDestroy } from 'svelte';
  import { open } from '@tauri-apps/plugin-dialog';
  import { listen } from '@tauri-apps/api/event';
  import {
    lockItem,
    unlockItem,
    unlockTlockFile,
    getAppState,
    saveSettings,
    migrateToTlock,
    isLegacyKeyFile,
    isTlockFile,
    onLockProgress,
    onUnlockProgress,
    openInExplorer
  } from './lib/api/tauri';

  let unlistenFileDrop = null;
  let unlistenFileDropHover = null;
  let unlistenFileDropCancelled = null;
  let unlistenLockProgress = null;
  let unlistenUnlockProgress = null;
  let tickInterval = null;

  // All state comes from backend
  let lockedItems = [];
  let vaults = [];
  let tick = 0; // Used to force re-render of time displays

  // UI-only state (ephemeral)
  let isDragging = false;
  let showLockModal = false;
  let showSettings = false;
  let showMigrationModal = false;
  let pendingFiles = [];
  let pendingMigrationFiles = [];
  let timeMode = 'for';
  let forDuration = '';
  let untilDate = '';
  let untilTime = '';
  let showTimePicker = false;
  let selectedVault = null;
  let message = null;
  let isLoading = true;
  let deleteOriginalFiles = false;
  let deleteMigrationOriginals = false;

  // Progress state for lock operations
  let isLocking = false;
  let lockProgress = {
    stage: 'compressing',
    progress: 0,
    currentFile: '',
    bytesProcessed: 0,
    totalBytes: 0
  };

  // Progress state for unlock operations
  let isUnlocking = false;
  let unlockProgress = {
    stage: 'decrypting',
    progress: 0,
    currentFile: '',
    bytesProcessed: 0,
    totalBytes: 0
  };
  let unlockingItemId = null;

  // Track newly unlocked items in this session - maps item ID to output path
  // Used as fallback before backend state is refreshed
  let sessionUnlockedItems = {};

  // Computed: sorted items (unlocked first, then by time remaining)
  $: sortedItems = [...lockedItems].sort((a, b) => {
    // Ready items first
    if (a.isReady && !b.isReady) return -1;
    if (!a.isReady && b.isReady) return 1;
    // Then by unlock time (soonest first)
    return new Date(a.unlocks) - new Date(b.unlocks);
  });

  onMount(async () => {
    // Prevent default browser drag/drop behavior
    document.addEventListener('dragover', (e) => e.preventDefault());
    document.addEventListener('drop', (e) => e.preventDefault());

    // Listen for lock progress events from backend
    unlistenLockProgress = await onLockProgress((event) => {
      lockProgress = {
        stage: event.stage || 'compressing',
        progress: event.progress ?? 0,
        currentFile: event.currentFile || '',
        bytesProcessed: event.bytesProcessed || 0,
        totalBytes: event.totalBytes || 0
      };
    });

    // Listen for unlock progress events from backend
    unlistenUnlockProgress = await onUnlockProgress((event) => {
      unlockProgress = {
        stage: event.stage || 'decrypting',
        progress: event.progress ?? 0,
        currentFile: event.currentFile || '',
        bytesProcessed: event.bytesProcessed || 0,
        totalBytes: event.totalBytes || 0
      };
    });

    // Listen for Tauri drag-drop events (v2 event names)
    unlistenFileDrop = await listen('tauri://drag-drop', (event) => {
      const paths = event.payload?.paths || event.payload;
      if (paths && paths.length > 0) {
        handleDroppedFiles(paths);
        isDragging = false;
      }
    });

    unlistenFileDropHover = await listen('tauri://drag-over', () => {
      isDragging = true;
    });

    unlistenFileDropCancelled = await listen('tauri://drag-leave', () => {
      isDragging = false;
    });

    // Load all state from backend
    await refreshState();
    isLoading = false;

    // Start tick interval for real-time countdown updates
    tickInterval = setInterval(() => {
      tick++;
      // Also refresh isReady status
      lockedItems = lockedItems.map(item => ({
        ...item,
        isReady: new Date(item.unlocks) <= new Date()
      }));
    }, 1000);
  });

  onDestroy(() => {
    if (unlistenFileDrop) unlistenFileDrop();
    if (unlistenFileDropHover) unlistenFileDropHover();
    if (unlistenFileDropCancelled) unlistenFileDropCancelled();
    if (unlistenLockProgress) unlistenLockProgress();
    if (unlistenUnlockProgress) unlistenUnlockProgress();
    if (tickInterval) clearInterval(tickInterval);
  });

  // Handle dropped files - detect legacy .key.md files for migration
  function handleDroppedFiles(paths) {
    const legacyFiles = paths.filter(p => isLegacyKeyFile(p));
    const regularFiles = paths.filter(p => !isLegacyKeyFile(p) && !isTlockFile(p));

    // If there are legacy .key.md files, offer migration
    if (legacyFiles.length > 0) {
      pendingMigrationFiles = legacyFiles;
      showMigrationModal = true;
    }

    // Regular files go to the lock modal
    if (regularFiles.length > 0) {
      pendingFiles = regularFiles;
      showLockModal = true;
    }
  }

  // Single function to refresh all state from backend
  async function refreshState() {
    try {
      const state = await getAppState();
      lockedItems = state.lockedItems;
      vaults = state.settings.vaults;
    } catch (error) {
      console.error('Failed to refresh state:', error);
      showMessage('error', 'Failed to load data');
    }
  }

  function handleDrop(event) {
    isDragging = false;
    event.preventDefault();
  }

  async function handleFileSelect() {
    try {
      const selected = await open({ multiple: true, directory: false });
      if (selected) {
        pendingFiles = Array.isArray(selected) ? selected : [selected];
        showLockModal = true;
      }
    } catch (error) {
      showMessage('error', 'Failed to select files');
    }
  }

  function parseDuration(input) {
    const match = input.trim().toLowerCase().match(/^(\d+)\s*(s|sec|seconds?|m|min|minutes?|h|hrs?|hours?|d|days?|w|weeks?|mo|months?|y|years?)$/);
    if (!match) return null;

    const num = parseInt(match[1]);
    const unit = match[2][0];
    const multipliers = { s: 1000, m: 60000, h: 3600000, d: 86400000, w: 604800000 };

    if (unit === 'y') return num * 365 * 86400000;
    if (match[2].startsWith('mo')) return num * 30 * 86400000;
    return num * (multipliers[unit] || 86400000);
  }

  async function handleLock() {
    let unlockDate;

    if (timeMode === 'for') {
      const ms = parseDuration(forDuration);
      if (!ms) {
        showMessage('error', 'Invalid duration format');
        return;
      }
      unlockDate = new Date(Date.now() + ms);
    } else {
      if (!untilDate) {
        showMessage('error', 'Please select a date');
        return;
      }
      const dateStr = untilTime ? `${untilDate}T${untilTime}` : `${untilDate}T00:00`;
      unlockDate = new Date(dateStr);
      if (unlockDate <= new Date()) {
        showMessage('error', 'Date must be in the future');
        return;
      }
    }

    try {
      // Start progress tracking
      isLocking = true;
      lockProgress = { stage: 'compressing', progress: 0, currentFile: '', bytesProcessed: 0, totalBytes: 0 };

      const unlockTimeISO = unlockDate.toISOString();
      const lockOptions = { deleteOriginal: deleteOriginalFiles };

      // Process files sequentially to show progress for each
      const results = [];
      for (const filePath of pendingFiles) {
        lockProgress.currentFile = getFileName(filePath);
        const result = await lockItem(filePath, unlockTimeISO, selectedVault, lockOptions);
        results.push(result);
      }

      isLocking = false;

      const failures = results.filter(r => !r.success);
      if (failures.length > 0) {
        const errorMsg = failures[0].error || 'Unknown error';
        showMessage('error', `Failed to lock ${failures.length} file(s): ${errorMsg}`);
      } else {
        const msg = deleteOriginalFiles
          ? `Locked ${pendingFiles.length} file(s) and deleted originals`
          : `Locked ${pendingFiles.length} file(s)`;
        showMessage('success', msg);
      }
      closeLockModal();
      await refreshState();
    } catch (error) {
      isLocking = false;
      showMessage('error', `Error: ${error.message || error}`);
      closeLockModal();
    }
  }

  // Handle migration of legacy .key.md files to .7z.tlock format
  async function handleMigration() {
    try {
      const results = await Promise.all(
        pendingMigrationFiles.map(keyPath => migrateToTlock(keyPath, deleteMigrationOriginals))
      );

      const failures = results.filter(r => !r.success);
      if (failures.length > 0) {
        const errorMsg = failures[0].error || 'Unknown error';
        showMessage('error', `Failed to migrate ${failures.length} file(s): ${errorMsg}`);
      } else {
        showMessage('success', `Migrated ${pendingMigrationFiles.length} file(s) to new format`);
      }
      closeMigrationModal();
      await refreshState();
    } catch (error) {
      showMessage('error', `Migration error: ${error.message || error}`);
      closeMigrationModal();
    }
  }

  function closeMigrationModal() {
    showMigrationModal = false;
    pendingMigrationFiles = [];
    deleteMigrationOriginals = false;
  }

  // Get the unlocked path for an item (from backend or session cache)
  function getUnlockedPath(item) {
    return item.unlockedPath || sessionUnlockedItems[item.id] || null;
  }

  async function handleUnlock(item) {
    // Check if already unlocked - just open the directory
    const existingUnlockedPath = getUnlockedPath(item);
    if (existingUnlockedPath) {
      try {
        await openInExplorer(existingUnlockedPath);
      } catch (error) {
        showMessage('error', `Failed to open directory: ${error.message || error}`);
      }
      return;
    }

    if (!item.isReady) {
      showMessage('error', 'This item is still locked');
      return;
    }

    try {
      // Start unlock progress
      isUnlocking = true;
      unlockingItemId = item.id;
      unlockProgress = { stage: 'decrypting', progress: 0, currentFile: '', bytesProcessed: 0, totalBytes: 0 };

      let result;

      // Use different unlock method based on format
      if (item.isLegacyFormat || !item.tlockPath) {
        // Legacy format: .key.md + .7z
        result = await unlockItem(item.keyPath);
      } else {
        // New format: .7z.tlock
        result = await unlockTlockFile(item.tlockPath);
      }

      isUnlocking = false;
      unlockingItemId = null;

      if (result.success) {
        // Track the unlocked item in session (backend will pick it up on next refresh)
        sessionUnlockedItems[item.id] = result.outputPath;
        sessionUnlockedItems = sessionUnlockedItems; // Trigger reactivity

        showMessage('success', 'Unlocked successfully! Opening folder...');
        await refreshState();

        // Open the output directory in file explorer
        try {
          await openInExplorer(result.outputPath);
        } catch (error) {
          console.error('Failed to open directory:', error);
          showMessage('error', `Unlocked but failed to open folder: ${error}`);
        }
      } else {
        showMessage('error', result.error || 'Failed to unlock');
      }
    } catch (error) {
      isUnlocking = false;
      unlockingItemId = null;
      showMessage('error', `Error: ${error.message || error}`);
    }
  }

  function closeLockModal() {
    showLockModal = false;
    pendingFiles = [];
    forDuration = '';
    untilDate = '';
    untilTime = '';
    showTimePicker = false;
    deleteOriginalFiles = false;
    isLocking = false;
    lockProgress = { stage: 'compressing', progress: 0, currentFile: '', bytesProcessed: 0, totalBytes: 0 };
  }

  function setQuickDuration(val) {
    forDuration = val;
  }

  function showMessage(type, text) {
    message = { type, text };
    setTimeout(() => message = null, 3000);
  }

  async function addVault() {
    try {
      const selected = await open({ directory: true, multiple: false });
      if (selected && !vaults.includes(selected)) {
        const newVaults = [...vaults, selected];
        await saveSettings({ vaults: newVaults });
        await refreshState();
      }
    } catch (error) {
      showMessage('error', 'Failed to add vault');
    }
  }

  async function removeVault(index) {
    try {
      const newVaults = vaults.filter((_, i) => i !== index);
      await saveSettings({ vaults: newVaults });
      await refreshState();
    } catch (error) {
      showMessage('error', 'Failed to remove vault');
    }
  }

  // Format the unlock time display (tick dependency ensures re-render)
  function formatUnlockTime(unlockDate, _tick) {
    const now = new Date();
    const unlock = new Date(unlockDate);
    const diff = unlock - now;

    if (diff <= 0) return 'Ready to unlock';

    const days = Math.floor(diff / 86400000);
    const hours = Math.floor((diff % 86400000) / 3600000);
    const mins = Math.floor((diff % 3600000) / 60000);
    const secs = Math.floor((diff % 60000) / 1000);

    // Format the date/time
    const dateStr = unlock.toLocaleDateString('en-US', {
      month: 'short',
      day: 'numeric',
      year: unlock.getFullYear() !== now.getFullYear() ? 'numeric' : undefined
    });
    const timeStr = unlock.toLocaleTimeString('en-US', {
      hour: 'numeric',
      minute: '2-digit',
      hour12: true
    });

    // If less than 1 minute, show seconds
    if (days === 0 && hours === 0 && mins === 0) {
      return `${secs}s remaining`;
    }

    // If less than 1 hour, show minutes and seconds
    if (days === 0 && hours === 0) {
      return `${mins}m ${secs}s remaining`;
    }

    // If less than 24 hours, show hours and minutes
    if (days === 0) {
      return `${hours}h ${mins}m remaining`;
    }

    // Otherwise show date/time + time remaining
    const remaining = `${days}d ${hours}h`;
    return `${dateStr} at ${timeStr} · ${remaining}`;
  }

  // Get clean display name from item
  function getDisplayName(item) {
    let name = item.name || '';
    // Remove common extensions for both old and new formats
    name = name.replace(/\.7z\.tlock$/i, '');
    name = name.replace(/\.tlock$/i, '');
    name = name.replace(/\.key\.md$/i, '');
    name = name.replace(/\.md$/i, '');
    name = name.replace(/\.7z$/i, '');
    name = name.replace(/\.zip$/i, '');
    return name || 'Unknown';
  }

  // Format bytes to human readable size
  function formatBytes(bytes) {
    if (!bytes || bytes === 0) return '';
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(1024));
    return `${(bytes / Math.pow(1024, i)).toFixed(1)} ${units[i]}`;
  }

  // Get progress stage label for locking
  function getProgressStageLabel(stage) {
    switch (stage) {
      case 'compressing': return 'Compressing';
      case 'encrypting': return 'Encrypting';
      case 'finalizing': return 'Finalizing';
      default: return 'Processing';
    }
  }

  // Get progress stage label for unlocking
  function getUnlockProgressStageLabel(stage) {
    switch (stage) {
      case 'decrypting': return 'Decrypting';
      case 'extracting': return 'Extracting';
      case 'finalizing': return 'Finalizing';
      default: return 'Processing';
    }
  }

  // Get vault name from file path (works for both formats)
  function getVaultName(item) {
    // Use tlockPath for new format, keyPath for legacy
    const path = item.tlockPath || item.keyPath || item.archivePath;
    if (!path) return null;
    // Extract parent directory name
    const parts = path.split(/[\\/]/);
    if (parts.length >= 2) {
      return parts[parts.length - 2];
    }
    return null;
  }

  function getFileName(path) {
    return path?.split(/[\\/]/).pop() || 'Unknown';
  }
</script>

<main class="h-screen flex flex-col p-4">
  <!-- Header -->
  <header class="flex items-center justify-between mb-4">
    <h1 class="text-sm font-medium text-white/80 flex items-center gap-2">
      <span class="text-base">🔐</span> Time Locker
    </h1>
    <button class="icon-btn" on:click={() => showSettings = true} title="Settings">
      <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5" d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z"/>
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"/>
      </svg>
    </button>
  </header>

  <!-- Message Toast -->
  {#if message}
    <div class="mb-3 px-3 py-2 rounded-lg text-xs {message.type === 'error' ? 'bg-red-500/15 text-red-400' : 'bg-emerald-500/15 text-emerald-400'}">
      {message.text}
    </div>
  {/if}

  <!-- Drop Zone -->
  <div
    role="button"
    tabindex="0"
    class="drop-zone mb-4 {isDragging ? 'dragging' : ''}"
    on:dragenter={() => isDragging = true}
    on:dragleave={() => isDragging = false}
    on:drop={handleDrop}
    on:click={handleFileSelect}
    on:keypress={(e) => e.key === 'Enter' && handleFileSelect()}
  >
    <div class="text-2xl mb-2 opacity-70">📁</div>
    <p class="text-white/60 text-xs">Drop files to lock</p>
    <p class="text-white/30 text-[10px] mt-1">or click to browse</p>
  </div>

  <!-- Locked Items List -->
  <div class="flex-1 min-h-0 glass-panel overflow-hidden flex flex-col">
    <div class="px-3 py-2 border-b border-white/[0.06]">
      <p class="section-label mb-0">Locked Items</p>
    </div>

    <div class="flex-1 overflow-y-auto p-2">
      {#if isLoading}
        <div class="text-center py-8 text-white/30 text-xs">
          Loading...
        </div>
      {:else if sortedItems.length === 0}
        <div class="text-center py-8 text-white/30 text-xs">
          No locked items yet
        </div>
      {:else}
        {#each sortedItems as item}
          {@const vaultName = getVaultName(item)}
          {@const isItemUnlocking = isUnlocking && unlockingItemId === item.id}
          {@const isItemUnlocked = !!(item.unlockedPath || sessionUnlockedItems[item.id])}
          <div
            class="file-row {item.isReady ? 'ready' : ''} {isItemUnlocked ? 'unlocked-item' : ''}"
            on:click={() => handleUnlock(item)}
            on:keypress={(e) => e.key === 'Enter' && handleUnlock(item)}
            role="button"
            tabindex="0"
          >
            <!-- Lock/Unlock Icon -->
            <div class="lock-icon {isItemUnlocked ? 'opened' : item.isReady ? 'unlocked' : 'locked'}">
              {#if isItemUnlocking}
                <!-- Spinner icon for unlocking -->
                <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
                  <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="2"/>
                  <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"/>
                </svg>
              {:else if isItemUnlocked}
                <!-- Folder open icon -->
                <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5" d="M5 19a2 2 0 01-2-2V7a2 2 0 012-2h4l2 2h4a2 2 0 012 2v1M5 19h14a2 2 0 002-2v-5a2 2 0 00-2-2H9a2 2 0 00-2 2v5a2 2 0 01-2 2z"/>
                </svg>
              {:else if item.isReady}
                <!-- Unlocked icon -->
                <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5" d="M8 11V7a4 4 0 118 0m-4 8v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2z"/>
                </svg>
              {:else}
                <!-- Locked icon -->
                <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5" d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z"/>
                </svg>
              {/if}
            </div>

            <div class="flex-1 min-w-0">
              <div class="flex items-center gap-1.5">
                <p class="text-xs text-white/90 truncate font-medium">{getDisplayName(item)}</p>
                {#if item.isLegacyFormat}
                  <span class="legacy-badge">Legacy</span>
                {/if}
              </div>
              {#if isItemUnlocking}
                <!-- Progress bar for this item -->
                <div class="mt-1">
                  <div class="flex items-center gap-2">
                    <div class="flex-1 progress-container-inline">
                      <div class="progress-bar" style="width: {unlockProgress.progress}%"></div>
                    </div>
                    <span class="text-[9px] text-white/50">{getUnlockProgressStageLabel(unlockProgress.stage)}</span>
                  </div>
                </div>
              {:else}
                <p class="text-[10px] text-white/40 truncate">
                  {#if isItemUnlocked}
                    Click to open folder
                  {:else}
                    {formatUnlockTime(item.unlocks, tick)}{#if vaultName} · {vaultName}{/if}{#if item.metadata?.compressedSize} · {formatBytes(item.metadata.compressedSize)}{/if}
                  {/if}
                </p>
              {/if}
            </div>

            <!-- Status badge -->
            {#if isItemUnlocked}
              <span class="status-badge opened">Opened</span>
            {:else if item.isReady}
              <span class="status-badge ready">Ready</span>
            {/if}
          </div>
        {/each}
      {/if}
    </div>
  </div>
</main>

<!-- Lock Modal -->
{#if showLockModal}
  <div class="modal-overlay" role="dialog" aria-modal="true" on:click|self={!isLocking ? closeLockModal : null} on:keydown={(e) => e.key === 'Escape' && !isLocking && closeLockModal()}>
    <div class="modal-content">
      {#if isLocking}
        <!-- Progress View -->
        <div class="text-center py-4">
          <div class="mb-3">
            <div class="text-3xl mb-2">
              {#if lockProgress.stage === 'compressing'}
                <span class="animate-pulse">&#128451;</span>
              {:else if lockProgress.stage === 'encrypting'}
                <span class="animate-pulse">&#128274;</span>
              {:else}
                <span class="animate-pulse">&#128190;</span>
              {/if}
            </div>
            <h2 class="text-sm font-medium text-white/90">{getProgressStageLabel(lockProgress.stage)}</h2>
          </div>

          {#if lockProgress.currentFile}
            <p class="text-[10px] text-white/50 truncate mb-2">{lockProgress.currentFile}</p>
          {/if}

          <!-- Progress Bar -->
          <div class="progress-container mb-2">
            <div class="progress-bar" style="width: {lockProgress.progress}%"></div>
          </div>

          <div class="flex justify-between text-[10px] text-white/40">
            <span>{lockProgress.progress.toFixed(0)}%</span>
            {#if lockProgress.bytesProcessed && lockProgress.totalBytes}
              <span>{formatBytes(lockProgress.bytesProcessed)} / {formatBytes(lockProgress.totalBytes)}</span>
            {/if}
          </div>
        </div>
      {:else}
        <!-- Normal Lock Form -->
        <div class="flex items-center justify-between mb-4">
          <h2 class="text-sm font-medium text-white/90">Lock {pendingFiles.length} file{pendingFiles.length > 1 ? 's' : ''}</h2>
          <button class="icon-btn" on:click={closeLockModal}>
            <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
            </svg>
          </button>
        </div>

        <!-- Time Mode Toggle -->
        <div class="flex gap-1 mb-4 p-1 bg-white/[0.03] rounded-lg">
          <button class="tab-btn flex-1 {timeMode === 'for' ? 'active' : ''}" on:click={() => timeMode = 'for'}>
            For
          </button>
          <button class="tab-btn flex-1 {timeMode === 'until' ? 'active' : ''}" on:click={() => timeMode = 'until'}>
            Until
          </button>
        </div>

        {#if timeMode === 'for'}
          <!-- Duration Input -->
          <div class="mb-3">
            <input
              type="text"
              class="glass-input"
              placeholder="e.g. 30m, 2h, 7d"
              bind:value={forDuration}
            />
          </div>
          <div class="flex flex-wrap gap-1.5 mb-4">
            <button class="hint-chip" on:click={() => setQuickDuration('30s')}>30s</button>
            <button class="hint-chip" on:click={() => setQuickDuration('5m')}>5m</button>
            <button class="hint-chip" on:click={() => setQuickDuration('1h')}>1h</button>
            <button class="hint-chip" on:click={() => setQuickDuration('1d')}>1d</button>
            <button class="hint-chip" on:click={() => setQuickDuration('7d')}>7d</button>
            <button class="hint-chip" on:click={() => setQuickDuration('30d')}>30d</button>
            <button class="hint-chip" on:click={() => setQuickDuration('1y')}>1y</button>
          </div>
        {:else}
          <!-- Date Picker -->
          <div class="mb-3">
            <div class="flex gap-2">
              <input
                type="date"
                class="glass-input flex-1"
                bind:value={untilDate}
              />
              <button
                class="icon-btn {showTimePicker ? 'bg-white/[0.08]' : ''}"
                on:click={() => showTimePicker = !showTimePicker}
                title="Set time"
              >
                <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5" d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z"/>
                </svg>
              </button>
            </div>
          </div>
          {#if showTimePicker}
            <div class="mb-4">
              <input
                type="time"
                class="glass-input"
                bind:value={untilTime}
              />
            </div>
          {/if}
        {/if}

        <!-- Vault Selection (if multiple) -->
        {#if vaults.length > 0}
          <div class="mb-4">
            <p class="section-label">Save to vault</p>
            <select class="glass-input" bind:value={selectedVault}>
              <option value={null}>Current directory</option>
              {#each vaults as vault, i}
                <option value={vault}>{getFileName(vault)}</option>
              {/each}
            </select>
          </div>
        {/if}

        <!-- Delete Original Files Option -->
        <div class="mb-4">
          <label class="flex items-center gap-2 cursor-pointer">
            <input
              type="checkbox"
              class="checkbox-input"
              bind:checked={deleteOriginalFiles}
            />
            <span class="text-xs text-white/70">Delete original files after locking</span>
          </label>
          {#if deleteOriginalFiles}
            <p class="text-[10px] text-amber-400/70 mt-1 ml-5">Warning: Original files will be permanently deleted</p>
          {/if}
        </div>

        <button class="glass-button primary w-full" on:click={handleLock}>
          Lock
        </button>
      {/if}
    </div>
  </div>
{/if}

<!-- Settings Modal -->
{#if showSettings}
  <div class="modal-overlay" role="dialog" aria-modal="true" on:click|self={() => showSettings = false} on:keydown={(e) => e.key === 'Escape' && (showSettings = false)}>
    <div class="modal-content">
      <div class="flex items-center justify-between mb-4">
        <h2 class="text-sm font-medium text-white/90">Settings</h2>
        <button class="icon-btn" on:click={() => showSettings = false}>
          <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
          </svg>
        </button>
      </div>

      <p class="section-label">Vaults</p>
      <p class="text-[10px] text-white/40 mb-3">Folders where locked files are stored</p>

      <div class="space-y-2 mb-4 max-h-40 overflow-y-auto">
        {#if vaults.length === 0}
          <p class="text-xs text-white/30 py-2">No vaults added. Using current directory.</p>
        {:else}
          {#each vaults as vault, i}
            <div class="flex items-center gap-2 px-2 py-1.5 bg-white/[0.03] rounded-lg">
              <span class="text-xs text-white/60 truncate flex-1">{getFileName(vault)}</span>
              <button class="icon-btn" on:click={() => removeVault(i)}>
                <svg class="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
                </svg>
              </button>
            </div>
          {/each}
        {/if}
      </div>

      <button class="glass-button w-full" on:click={addVault}>
        + Add Vault
      </button>
    </div>
  </div>
{/if}

<!-- Migration Modal -->
{#if showMigrationModal}
  <div class="modal-overlay" role="dialog" aria-modal="true" on:click|self={closeMigrationModal} on:keydown={(e) => e.key === 'Escape' && closeMigrationModal()}>
    <div class="modal-content">
      <div class="flex items-center justify-between mb-4">
        <h2 class="text-sm font-medium text-white/90">Migrate Legacy Files</h2>
        <button class="icon-btn" on:click={closeMigrationModal}>
          <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
          </svg>
        </button>
      </div>

      <div class="mb-4">
        <p class="text-xs text-white/70 mb-2">
          Found {pendingMigrationFiles.length} legacy .key.md file{pendingMigrationFiles.length > 1 ? 's' : ''}.
        </p>
        <p class="text-[10px] text-white/50">
          Convert to the new .7z.tlock format for a cleaner single-file experience.
        </p>
      </div>

      <!-- Files to migrate -->
      <div class="mb-4 max-h-32 overflow-y-auto">
        {#each pendingMigrationFiles as file}
          <div class="flex items-center gap-2 px-2 py-1.5 bg-white/[0.03] rounded-lg mb-1">
            <span class="text-base">&#128196;</span>
            <span class="text-xs text-white/60 truncate flex-1">{getFileName(file)}</span>
          </div>
        {/each}
      </div>

      <!-- Delete originals option -->
      <div class="mb-4">
        <label class="flex items-center gap-2 cursor-pointer">
          <input
            type="checkbox"
            class="checkbox-input"
            bind:checked={deleteMigrationOriginals}
          />
          <span class="text-xs text-white/70">Delete original files after migration</span>
        </label>
        {#if deleteMigrationOriginals}
          <p class="text-[10px] text-amber-400/70 mt-1 ml-5">Warning: .key.md and .7z files will be deleted</p>
        {/if}
      </div>

      <div class="flex gap-2">
        <button class="glass-button flex-1" on:click={closeMigrationModal}>
          Cancel
        </button>
        <button class="glass-button primary flex-1" on:click={handleMigration}>
          Migrate
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  :global(body) { margin: 0; padding: 0; }
  :global(input[type="date"]), :global(input[type="time"]) { color-scheme: dark; }
  :global(input[type="date"]::-webkit-calendar-picker-indicator),
  :global(input[type="time"]::-webkit-calendar-picker-indicator) {
    filter: invert(0.5);
    cursor: pointer;
  }

  /* Progress bar styles */
  .progress-container {
    width: 100%;
    height: 6px;
    background: rgba(255, 255, 255, 0.05);
    border-radius: 3px;
    overflow: hidden;
  }

  .progress-bar {
    height: 100%;
    background: linear-gradient(90deg, #6366f1, #8b5cf6);
    border-radius: 3px;
    transition: width 0.3s ease;
  }

  /* Checkbox styles */
  .checkbox-input {
    appearance: none;
    -webkit-appearance: none;
    width: 16px;
    height: 16px;
    border: 1.5px solid rgba(255, 255, 255, 0.3);
    border-radius: 4px;
    background: rgba(255, 255, 255, 0.05);
    cursor: pointer;
    transition: all 0.15s ease;
    position: relative;
    flex-shrink: 0;
  }

  .checkbox-input:hover {
    border-color: rgba(255, 255, 255, 0.5);
  }

  .checkbox-input:checked {
    background: #6366f1;
    border-color: #6366f1;
  }

  .checkbox-input:checked::after {
    content: '';
    position: absolute;
    left: 4px;
    top: 1px;
    width: 5px;
    height: 9px;
    border: solid white;
    border-width: 0 2px 2px 0;
    transform: rotate(45deg);
  }

  /* Pulse animation for icons */
  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.5; }
  }

  .animate-pulse {
    animation: pulse 1.5s ease-in-out infinite;
  }

  /* Legacy badge for old format items */
  .legacy-badge {
    font-size: 9px;
    padding: 2px 6px;
    background: rgba(251, 146, 60, 0.15);
    color: rgb(251, 146, 60);
    border-radius: 4px;
    font-weight: 500;
  }

  /* Inline progress bar for unlock */
  .progress-container-inline {
    height: 4px;
    background: rgba(255, 255, 255, 0.08);
    border-radius: 2px;
    overflow: hidden;
  }

  /* Opened/unlocked item styles */
  .file-row.unlocked-item {
    @apply bg-sky-500/[0.05];
  }

  .file-row.unlocked-item:hover {
    @apply bg-sky-500/[0.08];
  }

  .lock-icon.opened {
    @apply text-sky-400;
    background: linear-gradient(135deg, rgba(56, 189, 248, 0.2) 0%, rgba(14, 165, 233, 0.15) 100%);
  }

  .status-badge.opened {
    @apply bg-sky-500/20 text-sky-400;
  }

  /* Spin animation for loading */
  @keyframes spin {
    from { transform: rotate(0deg); }
    to { transform: rotate(360deg); }
  }

  .animate-spin {
    animation: spin 1s linear infinite;
  }
</style>

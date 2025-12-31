<script lang="ts">
  import { lockedItems } from '../stores/vault';
  import ItemCard from './ItemCard.svelte';

  async function handleUnlock(id: string) {
    const { invoke } = await import('@tauri-apps/api/core');

    try {
      await invoke('unlock_item', { itemId: id });

      lockedItems.update(items => items.filter(item => item.id !== id));
    } catch (error) {
      console.error('Failed to unlock item:', error);
      alert('Failed to unlock item. Please try again.');
    }
  }

  $: sortedItems = [...$lockedItems].sort((a, b) => a.unlockAt - b.unlockAt);
</script>

<div class="items-list">
  {#if sortedItems.length === 0}
    <div class="empty-state">
      <svg
        class="empty-icon"
        xmlns="http://www.w3.org/2000/svg"
        fill="none"
        viewBox="0 0 24 24"
        stroke="currentColor"
      >
        <path
          stroke-linecap="round"
          stroke-linejoin="round"
          stroke-width="2"
          d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z"
        />
      </svg>
      <h3 class="empty-title">No locked items</h3>
      <p class="empty-subtitle">
        Lock files or folders to get started
      </p>
    </div>
  {:else}
    <div class="items-grid">
      {#each sortedItems as item (item.id)}
        <ItemCard {item} onUnlock={handleUnlock} />
      {/each}
    </div>
  {/if}
</div>

<style>
  .items-list {
    min-height: 400px;
    padding: 1rem;
  }

  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    min-height: 400px;
    text-align: center;
    padding: 2rem;
  }

  .empty-icon {
    width: 80px;
    height: 80px;
    color: #444;
    margin-bottom: 1rem;
  }

  .empty-title {
    font-size: 1.25rem;
    font-weight: 600;
    color: #fff;
    margin: 0 0 0.5rem;
  }

  .empty-subtitle {
    font-size: 0.875rem;
    color: #888;
    margin: 0;
  }

  .items-grid {
    display: grid;
    gap: 1rem;
    grid-template-columns: 1fr;
  }

  @media (min-width: 640px) {
    .items-grid {
      grid-template-columns: repeat(2, 1fr);
    }
  }
</style>

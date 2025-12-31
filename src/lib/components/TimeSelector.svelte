<script lang="ts">
  import { createEventDispatcher } from 'svelte';

  const dispatch = createEventDispatcher();

  export let value: number = 0;

  const quickDurations = [
    { label: '5 min', value: 5 * 60 * 1000 },
    { label: '1 hour', value: 60 * 60 * 1000 },
    { label: '6 hours', value: 6 * 60 * 60 * 1000 },
    { label: '1 day', value: 24 * 60 * 60 * 1000 },
    { label: '1 week', value: 7 * 24 * 60 * 60 * 1000 },
    { label: '1 month', value: 30 * 24 * 60 * 60 * 1000 },
  ];

  let customDate = '';
  let customTime = '';

  function selectQuickDuration(duration: number) {
    value = Date.now() + duration;
    dispatch('change', { unlockAt: value });
  }

  function handleCustomDateTime() {
    if (customDate && customTime) {
      const dateTime = new Date(`${customDate}T${customTime}`);
      value = dateTime.getTime();
      dispatch('change', { unlockAt: value });
    }
  }
</script>

<div class="time-selector">
  <h3 class="selector-title">Lock Duration</h3>

  <div class="quick-buttons">
    {#each quickDurations as duration}
      <button
        class="quick-btn"
        on:click={() => selectQuickDuration(duration.value)}
      >
        {duration.label}
      </button>
    {/each}
  </div>

  <div class="divider">
    <span>or</span>
  </div>

  <div class="custom-datetime">
    <h4 class="custom-title">Unlock at specific time</h4>
    <div class="datetime-inputs">
      <input
        type="date"
        bind:value={customDate}
        on:change={handleCustomDateTime}
        class="datetime-input"
        min={new Date().toISOString().split('T')[0]}
      />
      <input
        type="time"
        bind:value={customTime}
        on:change={handleCustomDateTime}
        class="datetime-input"
      />
    </div>
  </div>

  {#if value > 0}
    <div class="selected-time">
      <p class="time-label">Will unlock on:</p>
      <p class="time-value">{new Date(value).toLocaleString()}</p>
    </div>
  {/if}
</div>

<style>
  .time-selector {
    padding: 1.5rem;
    background: #1a1a1a;
    border-radius: 8px;
    margin-bottom: 1rem;
  }

  .selector-title {
    font-size: 1rem;
    font-weight: 600;
    color: #fff;
    margin: 0 0 1rem;
  }

  .quick-buttons {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 0.5rem;
    margin-bottom: 1rem;
  }

  .quick-btn {
    padding: 0.75rem 1rem;
    background: #2a2a2a;
    color: #fff;
    border: 1px solid #333;
    border-radius: 6px;
    cursor: pointer;
    font-size: 0.875rem;
    transition: all 0.2s ease;
  }

  .quick-btn:hover {
    background: #6366f1;
    border-color: #6366f1;
  }

  .divider {
    text-align: center;
    margin: 1.5rem 0;
    position: relative;
  }

  .divider::before {
    content: '';
    position: absolute;
    left: 0;
    right: 0;
    top: 50%;
    height: 1px;
    background: #333;
  }

  .divider span {
    position: relative;
    background: #1a1a1a;
    padding: 0 1rem;
    color: #888;
    font-size: 0.875rem;
  }

  .custom-datetime {
    margin-bottom: 1rem;
  }

  .custom-title {
    font-size: 0.875rem;
    font-weight: 500;
    color: #aaa;
    margin: 0 0 0.5rem;
  }

  .datetime-inputs {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 0.5rem;
  }

  .datetime-input {
    padding: 0.75rem;
    background: #2a2a2a;
    color: #fff;
    border: 1px solid #333;
    border-radius: 6px;
    font-family: monospace;
    font-size: 0.875rem;
  }

  .datetime-input:focus {
    outline: none;
    border-color: #6366f1;
  }

  .selected-time {
    margin-top: 1rem;
    padding: 1rem;
    background: #2a2a2a;
    border-radius: 6px;
    border: 1px solid #6366f1;
  }

  .time-label {
    font-size: 0.75rem;
    color: #888;
    margin: 0 0 0.25rem;
  }

  .time-value {
    font-family: monospace;
    font-size: 0.875rem;
    color: #6366f1;
    margin: 0;
  }
</style>

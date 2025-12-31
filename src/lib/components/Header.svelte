<script lang="ts">
  export let title = 'Time Locker';
</script>

<header class="app-header">
  <div class="header-content">
    <div class="logo-section">
      <span class="logo-icon">🔐</span>
      <h1 class="app-title">{title}</h1>
    </div>
    <p class="app-tagline">Secure your files with time-based encryption</p>
  </div>
</header>

<style>
  .app-header {
    padding: 1.5rem 1rem 1rem;
    background: linear-gradient(180deg, #0f0f0f 0%, #1a1a1a 100%);
    border-bottom: 1px solid #333;
  }

  .header-content {
    max-width: 800px;
    margin: 0 auto;
  }

  .logo-section {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin-bottom: 0.5rem;
  }

  .logo-icon {
    font-size: 2rem;
  }

  .app-title {
    font-size: 1.5rem;
    font-weight: 700;
    color: #fff;
    margin: 0;
    background: linear-gradient(135deg, #6366f1 0%, #8b5cf6 100%);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    background-clip: text;
  }

  .app-tagline {
    font-size: 0.75rem;
    color: #888;
    margin: 0;
    padding-left: 2.75rem;
  }
</style>

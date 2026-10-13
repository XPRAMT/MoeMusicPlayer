<script lang="ts">
  interface Props {
    coverUrl: string | null;
    mainBackgroundUrl: string | null;
    onCoverError?: (url: string) => void;
  }

  let { coverUrl, mainBackgroundUrl, onCoverError = () => {} }: Props = $props();
</script>

<div class="now-playing-backdrop" data-testid="now-playing-backdrop" aria-hidden="true">
  {#if coverUrl}
    <img
      class="now-playing-cover-backdrop-image"
      data-backdrop-source="cover"
      src={coverUrl}
      alt=""
      onerror={() => onCoverError(coverUrl)}
    />
  {:else if mainBackgroundUrl}
    <img
      class="now-playing-main-backdrop-image"
      data-backdrop-source="main"
      src={mainBackgroundUrl}
      alt=""
    />
  {/if}
</div>

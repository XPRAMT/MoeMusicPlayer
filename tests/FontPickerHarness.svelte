<script lang="ts">
  import FontPicker from '../src/lib/FontPicker.svelte';

  let value = $state<string | null>(null);
  const chosen = ((globalThis as unknown as Record<string, unknown>).__FONT_PICKER_CHOSEN__ = []) as Array<string | null>;
</script>

<FontPicker {value} idPrefix="harness" onChange={(family) => { chosen.push(family); value = family; }} />

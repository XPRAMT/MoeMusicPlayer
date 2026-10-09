<script lang="ts">
  import AboutPage from '../src/lib/AboutPage.svelte';
  import UpdateCenter from '../src/lib/UpdateCenter.svelte';
  import { EMPTY_UPDATE_STATE } from '../src/lib/update-controller';
  import type { UpdateState } from '../src/lib/ipc';
  let updateState = $state<UpdateState>({ ...EMPTY_UPDATE_STATE });
  let updates: UpdateCenter;
</script>
<main style="padding: 20px; max-width: 800px; margin: auto">
  <button type="button" class="soft-button" id="before">前一個按鈕</button>
  <AboutPage platform="windows" {updateState} checkUpdate={() => updates.check()} showUpdate={() => updates.show()} />
</main>
<UpdateCenter enabled bind:this={updates} bind:updateState />

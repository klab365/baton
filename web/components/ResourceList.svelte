<script lang="ts">
import type { Snippet } from "svelte";

let { label, children }: { label: string; children: Snippet } = $props();
</script>

<ul class="resource-list" aria-label={label}>{@render children()}</ul>

<script lang="ts">
import type { Snippet } from "svelte";
import PageHeader from "./PageHeader.svelte";

let {
	title,
	description,
	children,
}: { title: string; description: string; children?: Snippet } = $props();
</script>

<section class="empty-state">
	<PageHeader eyebrow="Nothing here yet" {title} description={description} />
	{#if children}<div class="empty-state-action">{@render children()}</div>{/if}
</section>

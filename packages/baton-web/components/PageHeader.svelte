<script lang="ts">
import type { Snippet } from "svelte";

let {
	eyebrow,
	title,
	description,
	actions,
}: {
	eyebrow?: string;
	title: string;
	description?: string;
	actions?: Snippet;
} = $props();
</script>

<header class="page-header">
	<div>
		{#if eyebrow}<p class="eyebrow">{eyebrow}</p>{/if}
		<h1>{title}</h1>
		{#if description}<p class="lead">{description}</p>{/if}
	</div>
	{#if actions}<div class="page-actions">{@render actions()}</div>{/if}
</header>

<script lang="ts">
import type { Snippet } from "svelte";

let {
	title,
	children,
	actions,
}: { title: string; children: Snippet; actions?: Snippet } = $props();
</script>

<section class="detail-panel" aria-labelledby="detail-panel-title">
	<header><h2 id="detail-panel-title">{title}</h2>{#if actions}<div class="panel-actions">{@render actions()}</div>{/if}</header>
	<div class="detail-panel-content">{@render children()}</div>
</section>

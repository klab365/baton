<script lang="ts">
import type { Snippet } from "svelte";

let {
	title,
	id,
	children,
	actions,
}: { title: string; id?: string; children: Snippet; actions?: Snippet } =
	$props();
</script>

<section class="detail-panel" {id} aria-labelledby={id ? `${id}-title` : undefined}>
	<header><h2 id={id ? `${id}-title` : undefined}>{title}</h2>{#if actions}<div class="panel-actions">{@render actions()}</div>{/if}</header>
	<div class="detail-panel-content">{@render children()}</div>
</section>

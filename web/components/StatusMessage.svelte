<script lang="ts">
import type { Snippet } from "svelte";

let {
	tone = "info",
	title,
	children,
}: {
	tone?: "info" | "success" | "error" | "warning";
	title?: string;
	children: Snippet;
} = $props();
</script>

<div class="status status-{tone}" role={tone === "error" ? "alert" : "status"}>
	{#if title}<strong>{title}</strong>{/if}
	<div>{@render children()}</div>
</div>

<script lang="ts">
import type { Snippet } from "svelte";

let {
	variant = "primary",
	loading = false,
	type = "button",
	disabled = false,
	children,
	...rest
}: {
	variant?: "primary" | "secondary" | "danger" | "quiet";
	loading?: boolean;
	type?: "button" | "submit" | "reset";
	disabled?: boolean;
	children: Snippet;
} & Record<string, unknown> = $props();
</script>

<button class:button-danger={variant === "danger"} class:button-secondary={variant === "secondary"} class:button-quiet={variant === "quiet"} class="button" {type} disabled={disabled || loading} aria-busy={loading} {...rest}>
	{#if loading}<span aria-hidden="true">Working…</span>{:else}{@render children()}{/if}
</button>

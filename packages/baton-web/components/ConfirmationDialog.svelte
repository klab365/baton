<script lang="ts">
import AlertTriangle from "@lucide/svelte/icons/triangle-alert";
import type { Snippet } from "svelte";
import Button from "./Button.svelte";

let {
	open = $bindable(false),
	title,
	confirmLabel = "Delete",
	loading = false,
	onconfirm,
	children,
}: {
	open?: boolean;
	title: string;
	confirmLabel?: string;
	loading?: boolean;
	onconfirm: () => void;
	children: Snippet;
} = $props();
</script>

{#if open}
	<div class="dialog-backdrop" role="presentation">
		<div class="dialog" role="alertdialog" aria-modal="true" aria-labelledby="dialog-title" aria-describedby="dialog-description" tabindex="-1">
			<header class="dialog-header"><span class="dialog-icon" aria-hidden="true"><AlertTriangle size={19} /></span><div><span class="eyebrow">Destructive action</span><h2 id="dialog-title">{title}</h2></div></header>
			<div class="dialog-content" id="dialog-description">{@render children()}</div>
			<div class="dialog-actions">
				<Button variant="quiet" onclick={() => (open = false)}>Cancel</Button>
				<Button variant="danger" {loading} onclick={onconfirm}>{confirmLabel}</Button>
			</div>
		</div>
	</div>
{/if}

<script lang="ts">
import type { Snippet } from "svelte";

let {
	label,
	for: inputId,
	hint,
	error,
	required = false,
	children,
}: {
	label: string;
	for: string;
	hint?: string;
	error?: string;
	required?: boolean;
	children: Snippet<[{ describedBy: string; invalid: boolean }]>;
} = $props();
const hintId = $derived(hint ? `${inputId}-hint` : undefined);
const errorId = $derived(error ? `${inputId}-error` : undefined);
</script>

<div class="field">
	<label for={inputId}>{label}{#if required}<span aria-hidden="true"> *</span><span class="visually-hidden"> (required)</span>{/if}</label>
	{#if hint}<p class="field-hint" id={hintId}>{hint}</p>{/if}
	{@render children({ describedBy: [hintId, errorId].filter(Boolean).join(" "), invalid: Boolean(error) })}
	{#if error}<p class="field-error" id={errorId}>{error}</p>{/if}
</div>

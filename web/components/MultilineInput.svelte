<script lang="ts">
import { tick } from "svelte";
import Markdown from "./Markdown.svelte";

let {
	id,
	name,
	value = $bindable(""),
	rows = 5,
	placeholder,
	describedBy,
	invalid = false,
	disabled = false,
	required = false,
}: {
	id: string;
	name?: string;
	value?: string;
	rows?: number;
	placeholder?: string;
	describedBy?: string;
	invalid?: boolean;
	disabled?: boolean;
	required?: boolean;
} = $props();

let preview = $state(false);
let textarea: HTMLTextAreaElement;

$effect(() => {
	value;
	if (preview || !textarea) return;
	textarea.style.height = "auto";
	textarea.style.height = `${textarea.scrollHeight}px`;
});

async function showInvalidInput(event: Event) {
	if (!preview) return;
	event.preventDefault();
	preview = false;
	await tick();
	textarea.focus();
	textarea.reportValidity();
}
</script>

<div class="mode-switch" role="group" aria-label="Editor mode">
	<button type="button" aria-pressed={!preview} onclick={() => preview = false}>Text</button>
	<button type="button" aria-pressed={preview} onclick={() => preview = true}>Markdown Preview</button>
</div>
<textarea {id} {name} bind:this={textarea} bind:value {rows} {placeholder} aria-describedby={describedBy} aria-invalid={invalid} {disabled} {required} hidden={preview} oninvalid={showInvalidInput}></textarea>
{#if preview}
	<div class="preview" role="region" aria-label="Markdown preview" style:min-height={`${rows * 1.5}em`}>
		{#if value.trim()}<Markdown {value} />{:else}<p class="empty">Nothing to preview yet.</p>{/if}
	</div>
{/if}

<style>
	.mode-switch { display: flex; justify-content: flex-end; gap: 0.25rem; }
	.mode-switch button { color: inherit; background: transparent; border: 0; padding: 0.25rem 0.5rem; cursor: pointer; }
	.mode-switch button[aria-pressed="true"] { border-bottom: 2px solid currentColor; }
	.preview { border: 1px solid var(--border); border-radius: 0.5rem; padding: 0.75rem; background: var(--surface-muted); }
	.empty { margin: 0; color: var(--text-muted); }
	textarea { resize: none; overflow-y: hidden; }
</style>

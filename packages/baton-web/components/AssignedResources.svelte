<script lang="ts">
import Plus from "@lucide/svelte/icons/plus";
import Trash2 from "@lucide/svelte/icons/trash-2";
import { Dialog } from "bits-ui";
import Button from "./Button.svelte";
import ResourceSelect from "./ResourceSelect.svelte";

let {
	id,
	label,
	title,
	addLabel,
	emptyMessage,
	items,
	selected,
	disabled = false,
	onChange,
}: {
	id: string;
	label: string;
	title: string;
	addLabel: string;
	emptyMessage: string;
	items: { id: string; name?: string }[];
	selected: string[];
	disabled?: boolean;
	onChange: (selected: string[]) => void;
} = $props();

let open = $state(false);
let choice = $state("");
const available = $derived(items.filter((item) => !selected.includes(item.id)));
</script>

<header class="resources-header">
	<div><span class="eyebrow">Capabilities</span><h3>{title}</h3></div>
	<Button {id} variant="secondary" disabled={disabled || available.length === 0} onclick={() => { choice = ""; open = true; }}><Plus size={16} /> {addLabel}</Button>
</header>

<Dialog.Root bind:open>
	<Dialog.Portal>
		<Dialog.Overlay class="dialog-backdrop" />
		<Dialog.Content class="dialog resource-dialog">
			<header class="dialog-header"><div><span class="eyebrow">Capabilities</span><Dialog.Title>{addLabel}</Dialog.Title></div></header>
			<Dialog.Description class="dialog-content">Choose an available {title.toLowerCase().replace(/s$/, "")} to add.</Dialog.Description>
			<div class="resource-picker"><ResourceSelect id={`${id}-choice`} bind:value={choice} items={available} placeholder={`Select ${title.toLowerCase().replace(/s$/, "")}`} label={`Available ${title.toLowerCase()}`} /></div>
			<div class="dialog-actions"><Button variant="quiet" onclick={() => (open = false)}>Cancel</Button><Button disabled={!choice || !available.some((item) => item.id === choice)} onclick={() => { onChange([...selected, choice]); open = false; }}>{addLabel}</Button></div>
		</Dialog.Content>
	</Dialog.Portal>
</Dialog.Root>

{#if selected.length}
	<ul class="assigned-resources" aria-label={label}>
		{#each selected as value (value)}
			<li><span>{items.find((item) => item.id === value)?.name ?? value}</span><button type="button" aria-label={`Remove ${value}`} {disabled} onclick={() => onChange(selected.filter((item) => item !== value))}><Trash2 size={15} /></button></li>
		{/each}
	</ul>
{:else}
	<p class="empty">{emptyMessage}</p>
{/if}
<style>
	.assigned-resources { list-style: none; padding: 0; margin: 0 0 .65rem; border: 1px solid var(--border, #393a42); border-radius: .6rem; overflow: hidden; }
	.assigned-resources li { display: flex; align-items: center; justify-content: space-between; gap: 1rem; padding: .45rem .75rem; }
	.assigned-resources li + li { border-top: 1px solid var(--border, #393a42); }
	.assigned-resources button { display: grid; place-items: center; width: 2rem; height: 2rem; border: 0; border-radius: .4rem; background: transparent; color: #e3858b; cursor: pointer; }
	.assigned-resources button:hover:not(:disabled) { color: #ffadb2; background: #3a2024; }
	.empty { font-size: .85rem; color: var(--muted, #92939b); }
	.resources-header { display: flex; align-items: end; justify-content: space-between; gap: 1rem; margin-bottom: 1rem; }
	.resources-header h3 { margin: .15rem 0 0; font-size: .95rem; }
	:global(.resource-dialog) { position: fixed; z-index: 31; top: 50%; left: 50%; transform: translate(-50%, -50%); margin: 0; max-height: min(80vh, 40rem); display: flex; flex-direction: column; }
	.resource-picker { padding: 0 1.25rem 1.25rem; }
</style>

<script lang="ts">
import Box from "@lucide/svelte/icons/box";
import * as Select from "./ui/select";

let {
	id,
	value = $bindable(),
	items,
	placeholder,
	kind = "resource",
	describedBy,
	invalid = false,
	name,
	required = false,
	disabled = false,
	onValueChange,
}: {
	id: string;
	value?: string;
	items: { id: string; name?: string }[];
	placeholder: string;
	kind?: "agent" | "pipeline" | "resource";
	describedBy?: string;
	invalid?: boolean;
	name?: string;
	required?: boolean;
	disabled?: boolean;
	onValueChange?: (value: string) => void;
} = $props();
const Icon = $derived(Box);
const groupLabel = $derived(
	kind === "agent" ? "Configured agents" : "Pipelines",
);
</script>

<Select.Root bind:value {name} {required} {disabled} {onValueChange} type="single">
	<Select.Trigger {id} class="resource-select" aria-describedby={describedBy} aria-invalid={invalid}>
		{#if kind !== "agent"}<Icon size={16} />{/if}
		<Select.Value {placeholder}>
			{#snippet children()}
				{(items.find((item) => item.id === value)?.name ?? value) || placeholder}
			{/snippet}
		</Select.Value>
	</Select.Trigger>
	<Select.Content>
		<Select.Group>
			<Select.Label>{groupLabel}</Select.Label>
			{#each items as item}
				<Select.Item value={item.id} label={item.name ?? item.id}>
					{#if kind !== "agent"}<Icon size={15} />{/if}
					<span>{item.name ?? item.id}</span>
					{#if item.name}<small>{item.id}</small>{/if}
				</Select.Item>
			{/each}
		</Select.Group>
	</Select.Content>
</Select.Root>

<style>
	:global(.resource-select) {
		width: 100%;
		height: 3rem;
		min-height: 3rem;
		border-color: #aab4c6;
		border-radius: 0.4rem;
		background: #fff;
		color: #172033;
		font-family: inherit;
		font-size: inherit;
		font-weight: 600;
		padding: 0.65rem 0.75rem;
		transition: border-color 120ms ease, box-shadow 120ms ease;
	}
	:global(.resource-select:hover:not(:disabled)) { border-color: #66748b; }
	:global(.resource-select:focus-visible) {
		border-color: #3055b5;
		box-shadow: 0 0 0 3px rgb(48 85 181 / 15%);
		outline: 0;
	}
	:global(.resource-select:disabled) { background: #f7f8fb; }
	:global([data-slot="select-item"] small) { color: #69758b; font-size: 0.72rem; }
</style>

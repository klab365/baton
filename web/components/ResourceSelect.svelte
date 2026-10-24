<script lang="ts">
import Box from "@lucide/svelte/icons/box";
import Cpu from "@lucide/svelte/icons/cpu";
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
	kind?: "agent" | "pipeline" | "resource" | "provider" | "model";
	describedBy?: string;
	invalid?: boolean;
	name?: string;
	required?: boolean;
	disabled?: boolean;
	onValueChange?: (value: string) => void;
} = $props();
const Icon = $derived(kind === "agent" ? Cpu : Box);
const groupLabel = $derived(
	kind === "agent"
		? "Configured agents"
		: kind === "provider"
			? "Verified providers"
			: kind === "model"
				? "Available models"
				: "Pipelines",
);
</script>

<Select.Root bind:value {name} {required} {disabled} {onValueChange} type="single">
	<Select.Trigger {id} class="resource-select" aria-describedby={describedBy} aria-invalid={invalid}>
		<Icon size={16} />
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
					<Icon size={15} />
					<span>{item.name ?? item.id}</span>
					{#if item.name && kind !== "provider"}<small>{item.id}</small>{/if}
				</Select.Item>
			{/each}
		</Select.Group>
	</Select.Content>
</Select.Root>

<style>
	:global([data-slot="select-item"] small) { color: #69758b; font-size: 0.72rem; }
</style>

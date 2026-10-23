<script lang="ts">
let {
	id,
	name,
	value = $bindable(""),
	rows = 5,
	placeholder,
	describedBy,
	invalid = false,
	disabled = false,
}: {
	id: string;
	name: string;
	value?: string;
	rows?: number;
	placeholder?: string;
	describedBy?: string;
	invalid?: boolean;
	disabled?: boolean;
} = $props();
</script>

<textarea {id} {name} bind:value {rows} {placeholder} aria-describedby={describedBy} aria-invalid={invalid} {disabled}></textarea>

<script lang="ts">
import { parseRunActivity } from "$lib/run-activity";
let { entries }: { entries: { kind: string; content: string }[] } = $props();
const activity = $derived(
	entries
		.filter((entry) => entry.kind === "runtimeTranscript")
		.flatMap((entry) => parseRunActivity(entry.content)),
);
</script>

<details class="activity">
	<summary>Agent activity ({activity.length})</summary>
	<p class="muted">Recorded after the step finishes. Tool inputs and outputs may contain sensitive information.</p>
	{#if activity.length}
		<ol class="activity-list">{#each activity as item}<li><details><summary>{item.label}</summary><pre>{item.content}</pre></details></li>{/each}</ol>
	{:else}<p class="muted">No readable activity is available. See raw execution trace for other runtime data.</p>{/if}
</details>

<style>
	.activity { margin-top: 0.85rem; }
	.activity-list { display: grid; gap: 0.5rem; padding-left: 1.25rem; }
	.activity-list li { padding: 0; border: 0; background: transparent; }
	.activity summary { cursor: pointer; }
</style>

<script lang="ts">
import { goto } from "$app/navigation";
import Button from "../../../components/Button.svelte";
import DetailPanel from "../../../components/DetailPanel.svelte";
import PageHeader from "../../../components/PageHeader.svelte";
import StatusMessage from "../../../components/StatusMessage.svelte";
let { data } = $props();
let loading = $state(false);
let error = $state("");
async function cancel() {
	loading = true;
	try {
		const response = await fetch(`/api/runs/${data.run.id}/cancel`, {
			method: "POST",
		});
		if (!response.ok)
			throw new Error((await response.json()).error ?? "Could not cancel run.");
		await goto(`/runs/${data.run.id}`, { invalidateAll: true });
	} catch (e) {
		error = e instanceof Error ? e.message : "Could not cancel run.";
	} finally {
		loading = false;
	}
}
</script>
<svelte:head><title>Run · Baton</title></svelte:head>
<PageHeader eyebrow="Run" title={data.run.id} description={`Pipeline: ${data.run.pipelineId}`}>
	{#snippet actions()}{#if data.run.status === "running"}<Button {loading} variant="danger" onclick={cancel}>Cancel run</Button>{/if}{/snippet}
</PageHeader>
{#if error}<StatusMessage tone="error">{error}</StatusMessage>{/if}
<div class="task-detail-layout">
	<DetailPanel title="Status"><dl class="task-facts"><div><dt>Status</dt><dd><span class="task-status">{data.run.status}</span></dd></div><div><dt>Started</dt><dd>{new Date(data.run.startedAt).toLocaleString()}</dd></div><div><dt>Ended</dt><dd>{data.run.endedAt ? new Date(data.run.endedAt).toLocaleString() : "In progress"}</dd></div><div><dt>Duration</dt><dd>{data.run.durationMs == null ? "Unavailable" : `${data.run.durationMs} ms`}</dd></div></dl></DetailPanel>
	<DetailPanel title="Steps"><ol class="run-steps">{#each data.run.steps as step}<li><strong>{step.stepId}</strong> <span class="task-status">{step.status}</span>{#if step.input}<details><summary>Input</summary><pre>{step.input}</pre></details>{/if}{#if step.output}<details><summary>Output</summary><pre>{step.output}</pre></details>{/if}{#if step.error}<p class="field-error">{step.error}</p>{/if}</li>{/each}</ol></DetailPanel>
	<DetailPanel title="Events"><ol class="run-events">{#each data.run.events as event}<li><time datetime={event.occurredAt}>{new Date(event.occurredAt).toLocaleString()}</time> — {event.message ?? event.status}</li>{/each}</ol></DetailPanel>
</div>

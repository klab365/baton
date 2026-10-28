<script lang="ts">
import { goto } from "$app/navigation";
import Button from "../../../components/Button.svelte";
import DetailPanel from "../../../components/DetailPanel.svelte";
import PageHeader from "../../../components/PageHeader.svelte";
import StatusMessage from "../../../components/StatusMessage.svelte";
let { data } = $props();
let loading = $state(false);
let error = $state("");
let responseText = $state("");
const pipeline = $derived(
	data.run.configSnapshot?.pipelines.find(
		(p: { id: string }) => p.id === data.run.pipelineId,
	),
);
const humanActions = $derived(data.run.pendingActions ?? []);
async function act(outcome: string) {
	loading = true;
	error = "";
	try {
		const response = await fetch(`/api/runs/${data.run.id}/actions`, {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ outcome, response: responseText }),
		});
		if (!response.ok)
			throw new Error(
				(await response.json()).error ?? "Could not submit action.",
			);
		await goto(`/runs/${data.run.id}`, { invalidateAll: true });
	} catch (e) {
		error = e instanceof Error ? e.message : "Could not submit action.";
	} finally {
		loading = false;
	}
}
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
	{#snippet actions()}{#if ["running", "waitingHuman"].includes(data.run.status)}<Button {loading} variant="danger" onclick={cancel}>Cancel run</Button>{/if}{/snippet}
</PageHeader>
{#if error}<StatusMessage tone="error">{error}</StatusMessage>{/if}
{#if data.run.status === "waitingHuman"}
<DetailPanel title="Pending human action">
	<p>Waiting at {data.run.currentStep}. Only an explicit action resumes this run. Cancel to abort; there is no automatic approval or merge.</p>
	<label for="human-response">Response</label><textarea id="human-response" bind:value={responseText} disabled={loading}></textarea>
	{#each humanActions as outcome}<Button {loading} onclick={() => act(outcome)}>{outcome}</Button>{/each}
</DetailPanel>
{/if}
<div class="task-detail-layout">
	<DetailPanel title="Execution graph">
		<p>Current step: {data.run.currentStep ?? "Complete"}</p>
		{#if pipeline}<ol>{#each pipeline.steps as step, index}<li><strong>{step.id}</strong> {step.human ? "(human wait)" : "(agent)"}
		{#if step.transitions.length}<ul>{#each step.transitions as transition}<li>{transition.outcome} → {transition.to ?? "Complete"}</li>{/each}</ul>
		{:else} → {pipeline.steps[index + 1]?.id ?? "Complete"}{/if}</li>{/each}</ol>{:else}<p>Graph unavailable for legacy run.</p>{/if}
	</DetailPanel>
	<DetailPanel title="Status"><dl class="task-facts"><div><dt>Status</dt><dd><span class="task-status">{data.run.status}</span></dd></div><div><dt>Started</dt><dd>{new Date(data.run.startedAt).toLocaleString()}</dd></div><div><dt>Ended</dt><dd>{data.run.endedAt ? new Date(data.run.endedAt).toLocaleString() : "In progress"}</dd></div><div><dt>Duration</dt><dd>{data.run.durationMs == null ? "Unavailable" : `${data.run.durationMs} ms`}</dd></div></dl></DetailPanel>
	<DetailPanel title="Step attempts and handoffs"><ol class="run-steps">{#each data.run.steps.filter((step: { attempt: number }) => step.attempt > 0).sort((a: { attempt: number }, b: { attempt: number }) => a.attempt - b.attempt) as step}<li><strong>{step.attempt ? `Attempt ${step.attempt}: ` : ""}{step.stepId}</strong> <span class="task-status">{step.status}</span>{#if step.outcome}<p>Outcome: {step.outcome}</p>{/if}{#if step.selectedTransition}<p>Selected: {step.selectedTransition.outcome} → {step.selectedTransition.to ?? "Complete"}</p>{/if}{#if step.input}<details><summary>Input</summary><pre>{step.input}</pre></details>{/if}{#if step.output}<details><summary>Output</summary><pre>{step.output}</pre></details>{/if}{#if step.error}<p class="field-error">{step.error}</p>{/if}</li>{/each}</ol></DetailPanel>
	<DetailPanel title="Events"><ol class="run-events">{#each data.run.events as event}<li><time datetime={event.occurredAt}>{new Date(event.occurredAt).toLocaleString()}</time> — {event.message ?? event.status}</li>{/each}</ol></DetailPanel>
</div>

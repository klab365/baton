<script lang="ts">
import { goto } from "$app/navigation";
import Button from "../../../components/Button.svelte";
import DetailPanel from "../../../components/DetailPanel.svelte";
import MultilineInput from "../../../components/MultilineInput.svelte";
import PageHeader from "../../../components/PageHeader.svelte";
import StatusMessage from "../../../components/StatusMessage.svelte";
import StatusBadge from "../../../components/StatusBadge.svelte";
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
const completedSteps = $derived(
	data.run.steps.filter(
		(step: { status: string }) => step.status === "succeeded",
	).length,
);
const totalSteps = $derived(pipeline?.steps.length ?? data.run.steps.length);
const nextAction = $derived(
	data.run.status === "waitingHuman"
		? "Choose an outcome below to resume this run."
		: data.run.status === "running"
			? `Working on ${data.run.currentStep ?? "the next step"}.`
			: data.run.status === "failed"
				? "Review the failed step and execution trace."
				: data.run.status === "cancelled"
					? "This run was cancelled."
					: "Run complete.",
);
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
	error = "";
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
<PageHeader eyebrow="Pipeline run" title={data.run.id} description={`Pipeline: ${data.run.pipelineId}`}>
	{#snippet actions()}<a class="button button-secondary" href="/runs">All runs</a>{#if ["running", "waitingHuman"].includes(data.run.status)}<Button {loading} variant="danger" onclick={cancel}>Cancel run</Button>{/if}{/snippet}
</PageHeader>
{#if error}<StatusMessage tone="error">{error}</StatusMessage>{/if}

<section class="run-overview" aria-label="Run overview">
	<div><StatusBadge status={data.run.status} /><strong>{nextAction}</strong><span>{completedSteps} of {totalSteps} steps completed {data.run.currentStep ? `· Current: ${data.run.currentStep}` : ""}</span></div>
	<div class="run-overview-links"><a href="#steps">Step details</a><a href="#trace">Execution trace</a><a href="#events">Run events</a></div>
</section>

{#if data.run.status === "waitingHuman"}
<DetailPanel title="Action required">
	<p>Paused at <strong>{data.run.currentStep}</strong>. Your selected outcome resumes the pipeline; cancelling aborts it.</p>
	<label for="human-response">Response for the next step (optional)</label>
	<MultilineInput id="human-response" name="human-response" bind:value={responseText} disabled={loading} rows={4} placeholder="Add context for the next step" />
	<div class="page-actions">{#each humanActions as outcome}<Button {loading} onclick={() => act(outcome)}>{outcome}</Button>{/each}</div>
</DetailPanel>
{/if}

<div class="task-detail-layout">
	<DetailPanel title="Progress">
		{#if pipeline}<ol class="run-graph">{#each pipeline.steps as step}<li class:current={step.id === data.run.currentStep}><strong>{step.id}</strong><span>{step.human ? "Human decision" : "Agent step"}</span></li>{/each}</ol>{:else}<p>Step configuration is unavailable for this legacy run.</p>{/if}
	</DetailPanel>
	<DetailPanel title="Run status"><dl class="task-facts"><div><dt>Started</dt><dd>{new Date(data.run.startedAt).toLocaleString()}</dd></div><div><dt>Ended</dt><dd>{data.run.endedAt ? new Date(data.run.endedAt).toLocaleString() : "In progress"}</dd></div><div><dt>Duration</dt><dd>{data.run.durationMs == null ? "In progress" : `${data.run.durationMs} ms`}</dd></div><div><dt>Current step</dt><dd>{data.run.currentStep ?? "Complete"}</dd></div></dl></DetailPanel>
	<DetailPanel title="Step details" id="steps"><ol class="run-steps">{#each data.run.steps.filter((step: { attempt: number }) => step.attempt > 0).sort((a: { attempt: number }, b: { attempt: number }) => a.attempt - b.attempt) as step}<li><header><strong>Attempt {step.attempt}: {step.stepId}</strong><StatusBadge status={step.status} /></header>{#if step.input}<details><summary>Context received by this step</summary><pre>{step.input}</pre></details>{/if}{#if step.selectedTransition}<p><strong>Transition handoff</strong> — <code>{step.selectedTransition.outcome}</code> → {step.selectedTransition.to ?? "Complete"}. The summary below came from <code>report_outcome</code> and is sent to the next step.</p>{:else if step.output}<p><strong>Agent response</strong> — this step has no configured transition.</p>{/if}{#if step.output}<details open><summary>{step.selectedTransition ? "Handoff summary" : "Agent response"}</summary><pre>{step.output}</pre></details>{/if}{#if step.error}<p class="field-error"><strong>Step failed:</strong> {step.error}</p>{/if}</li>{/each}</ol></DetailPanel>
	<DetailPanel title="Execution trace" id="trace"><p class="muted">Persisted agent messages, tool calls and results, when supplied by the runtime. Older runs may not have a trace.</p>{#if data.run.trace?.length}<ol class="run-events">{#each data.run.trace as entry}<li><time datetime={entry.occurredAt}>{new Date(entry.occurredAt).toLocaleString()}</time> <strong>{entry.kind}</strong><details><summary>Show entry</summary><pre>{entry.content}</pre></details></li>{/each}</ol>{:else}<p>No persisted runtime trace is available for this run.</p>{/if}</DetailPanel>
	<DetailPanel title="Run events" id="events"><ol class="run-events">{#each data.run.events as event}<li><time datetime={event.occurredAt}>{new Date(event.occurredAt).toLocaleString()}</time> — {event.message ?? event.status}</li>{/each}</ol></DetailPanel>
</div>

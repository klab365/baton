<script lang="ts">
import { invalidate, invalidateAll } from "$app/navigation";
import { onMount } from "svelte";
import { runFailure } from "$lib/run-failure";
import { replaySteps } from "$lib/run-replay";
import Button from "../../../components/Button.svelte";
import DetailPanel from "../../../components/DetailPanel.svelte";
import MultilineInput from "../../../components/MultilineInput.svelte";
import Markdown from "../../../components/Markdown.svelte";
import PageHeader from "../../../components/PageHeader.svelte";
import RunActivity from "../../../components/RunActivity.svelte";
import StatusMessage from "../../../components/StatusMessage.svelte";
import StatusBadge from "../../../components/StatusBadge.svelte";
let { data } = $props();
let actionLoading = $state(false);
let cancelling = $state(false);
let error = $state("");
let responseText = $state("");
let connection = $state<"connecting" | "live" | "reconnecting">("connecting");
let refreshError = $state(false);
let clock = $state(Date.now());
onMount(() => {
	const clockInterval = window.setInterval(() => (clock = Date.now()), 1000);
	let refreshing = false;
	let pending = false;
	let stopped = false;
	const refresh = async () => {
		if (document.hidden || stopped) return;
		if (refreshing) {
			pending = true;
			return;
		}
		refreshing = true;
		try {
			do {
				pending = false;
				await invalidate(`/api/runs/${encodeURIComponent(data.run.id)}`);
				refreshError = false;
			} while (pending && !stopped);
		} catch {
			refreshError = true;
		} finally {
			refreshing = false;
		}
	};
	const source = new EventSource(
		`/api/runs/${encodeURIComponent(data.run.id)}/events`,
	);
	source.onopen = () => {
		connection = "live";
		void refresh();
	};
	source.onerror = () => {
		connection = "reconnecting";
	};
	source.addEventListener("update", () => void refresh());
	const interval = window.setInterval(() => {
		if (connection !== "live" || refreshError) void refresh();
	}, 10000);
	const visible = () => {
		if (!document.hidden) void refresh();
	};
	document.addEventListener("visibilitychange", visible);
	return () => {
		stopped = true;
		window.clearInterval(clockInterval);
		source.close();
		window.clearInterval(interval);
		document.removeEventListener("visibilitychange", visible);
	};
});
const pipeline = $derived(
	data.run.configSnapshot?.pipelines.find(
		(p: { id: string }) => p.id === data.run.pipelineId,
	),
);
const humanActions = $derived(data.run.pendingActions ?? []);
const attempts = $derived(replaySteps<any>(data.run.steps));
const legacyTraceMayContainSensitiveContent = $derived(
	(data.run.trace ?? []).some(
		(entry: { kind: string; content: string }) =>
			entry.kind === "runtimeTranscript" ||
			entry.content.includes('"systemPrompt"') ||
			entry.content.includes('"systemMessages"') ||
			entry.content.includes('"messages"'),
	),
);
const previousAttempt = $derived(attempts.at(-2));
const failure = $derived(runFailure(data.run));
function isHuman(stepId: string) {
	return (
		pipeline?.steps.some(
			(step: { id: string; type: string }) =>
				step.id === stepId && step.type === "human",
		) ?? false
	);
}
const nextAction = $derived(
	data.run.status === "waitingHuman"
		? "Choose an outcome below to resume this run."
		: data.run.status === "running"
			? `Working on ${data.run.currentStep ?? "the next step"}.`
			: data.run.status === "failed"
				? "Review the failed step and execution trace."
				: data.run.status === "cancelled"
					? "This run was cancelled."
					: data.run.status === "limitExhausted"
						? "The run reached its limit. Review the timeline."
						: data.run.status === "succeeded"
							? "Run complete."
							: `Run status: ${data.run.status}`,
);
async function act(outcome: string) {
	actionLoading = true;
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
		responseText = "";
		await invalidateAll();
	} catch (e) {
		error = e instanceof Error ? e.message : "Could not submit action.";
	} finally {
		actionLoading = false;
	}
}
async function cancel() {
	cancelling = true;
	error = "";
	try {
		const response = await fetch(`/api/runs/${data.run.id}/cancel`, {
			method: "POST",
		});
		if (!response.ok)
			throw new Error((await response.json()).error ?? "Could not cancel run.");
		await invalidateAll();
	} catch (e) {
		error = e instanceof Error ? e.message : "Could not cancel run.";
	} finally {
		cancelling = false;
	}
}
</script>

<svelte:head><title>{data.task?.title ?? "Pipeline run"} · Baton</title></svelte:head>
<PageHeader eyebrow="Pipeline run" title={data.task?.title ?? "Pipeline run"} description={`Pipeline: ${data.run.pipelineId}`}>
	{#snippet actions()}<a class="button button-secondary" href="/runs">All runs</a>{#if ["running", "waitingHuman"].includes(data.run.status)}<Button loading={cancelling} variant="danger" onclick={cancel}>Cancel run</Button>{/if}{/snippet}
</PageHeader>
{#if error}<StatusMessage tone="error">{error}</StatusMessage>{/if}

<section class="run-overview" aria-label="Run overview">
	<div><span class="overview-label">RUN STATUS</span><StatusBadge status={data.run.status} /><strong>{nextAction}</strong>{#if data.task}<span>Task: <a href={`/tasks/${encodeURIComponent(data.task.id)}`}>{data.task.title}</a></span>{:else if data.run.taskId}<span>Task: {data.run.taskId} (unavailable)</span>{/if}<span>{data.run.currentStep ? `Current step: ${data.run.currentStep} · ` : ""}Started {new Date(data.run.startedAt).toLocaleString()}</span></div>
	<div class="run-overview-links"><span role="status">{refreshError ? "Refresh failed · retrying" : connection === "live" ? "Live updates" : connection === "reconnecting" ? "Reconnecting · checking periodically" : "Connecting…"}</span><a href="#steps">View timeline ↓</a><a href="#diagnostics">Diagnostics ↓</a></div>
</section>

{#if failure}<div class="failure-summary"><StatusMessage tone="error" title={failure.title}><p>{failure.detail}</p>{#if failure.attempt}<a href={`#attempt-${failure.attempt}`}>View attempt {failure.attempt} in timeline</a>{:else}<a href="#diagnostics">View run diagnostics</a>{/if}</StatusMessage></div>{/if}

{#if data.run.status === "waitingHuman"}
<div class="action-panel"><DetailPanel title="Your input is needed">
	<p class="action-intro">The run is paused at <strong>{data.run.currentStep}</strong>. Choose an outcome to continue.</p>
	{#if previousAttempt?.output}<div class="action-context"><span>FROM {previousAttempt.stepId}{previousAttempt.outcome ? ` · ${previousAttempt.outcome}` : ""}</span><Markdown value={previousAttempt.output} /></div>{/if}
	<label for="human-response">Your response <span class="muted">(optional)</span></label>
	<MultilineInput id="human-response" name="human-response" bind:value={responseText} disabled={actionLoading || cancelling} rows={4} placeholder="Add context for the next step" />
	<div class="page-actions">{#each humanActions as outcome}<Button loading={actionLoading} disabled={cancelling} onclick={() => act(outcome)}>{outcome === "responded" ? "Send response" : outcome === "approved" ? "Approve" : outcome === "rejected" ? "Reject" : outcome}</Button>{/each}</div>
</DetailPanel></div>
{/if}

{#if pipeline}
<section class="pipeline-progress" aria-label="Configured pipeline steps">
	<ol>{#each pipeline.steps as configured (configured.id)}{@const latest = [...attempts].reverse().find((attempt: { stepId: string }) => attempt.stepId === configured.id)}{@const current = configured.id === data.run.currentStep && ["running", "waitingHuman"].includes(data.run.status)}<li><span class="progress-name">{configured.name || configured.id}</span><span class:current class:visited={latest && ["completed", "approved", "responded"].includes(latest.outcome)} class:failed={latest?.status === "failed" || latest?.outcome === "failed"} class:blocked={latest?.outcome === "blocked"} class="progress-dot" aria-hidden="true"></span><span class="progress-status">{current ? (data.run.status === "waitingHuman" ? "Waiting for you" : "Current") : latest ? (latest.outcome ?? latest.status) : "Not visited"}</span></li>{/each}</ol>
</section>
{/if}

<div class="task-detail-layout">
	<DetailPanel title="Replay · newest first" id="steps">
		<p class="muted">{data.run.session ? "External work session: runtime activity and handoff context may not be recorded." : "Only persisted results, transitions and handoff context are shown."}{#if attempts.some((step: { attempt: number }) => !step.attempt)} Some older steps lack attempt numbers; their saved order is shown in reverse.{/if}</p>
		{#if attempts.length}<ol class="run-steps timeline">{#each [...attempts].reverse() as step, index (index)}{@const stepTrace = (data.run.trace ?? []).filter((entry: { attempt?: number }) => step.attempt > 0 && entry.attempt === step.attempt)}<li id={`attempt-${step.attempt}`}><header><div class="step-heading"><span class="step-number">{step.attempt || "–"}</span><div><strong>{step.stepId}</strong><div class="step-meta">{isHuman(step.stepId) ? "Human decision" : step.agentId}</div></div></div><StatusBadge status={step.status} /></header><p class="muted">{step.startedAt ? `Started ${new Date(step.startedAt).toLocaleString()}` : "Start time unavailable"}{step.durationMs == null ? (step.status === "running" && step.startedAt ? ` · elapsed ${Math.floor(Math.max(0, clock - Date.parse(step.startedAt)) / 1000)}s` : step.status === "running" ? " · elapsed time unavailable" : " · duration unavailable") : ` · ${step.durationMs} ms`}</p><p class="muted">Model: {step.model ? `${step.model.provider}/${step.model.model}` : "not available"} · Input/output/cache tokens: {step.usage ? `${step.usage.inputTokens ?? "not available"}/${step.usage.outputTokens ?? "not available"}/${step.usage.cacheReadTokens ?? "not available"}/${step.usage.cacheWriteTokens ?? "not available"}` : "not available"} · Estimated cost: {step.usage?.estimatedCost ?? "not available"}{#if step.usage?.estimatedCost} ({step.usage.priceSource ?? "price source unavailable"}{step.usage.priceCheckedAt ? `, checked ${step.usage.priceCheckedAt}` : ""}){/if}</p>{#if step.outcome}<p class="step-route"><span>{step.outcome}</span>{#if step.selectedTransition}<span aria-hidden="true">→</span><span>{step.selectedTransition.to ?? "End run"}</span>{/if}</p>{/if}{#if step.output?.trim()}<div class="step-result"><span>{isHuman(step.stepId) ? "RESPONSE" : "RESULT"}</span>{#if step.output.length > 500}<Markdown value={step.output.slice(0, 350) + "…"} /><details><summary>Show full result</summary><Markdown value={step.output} /></details>{:else}<Markdown value={step.output} />{/if}</div>{:else if isHuman(step.stepId) && step.outcome}<p class="muted">No comment provided.</p>{/if}{#if step.error}<p class="field-error"><strong>Step failed:</strong> {step.error}</p>{/if}{#if stepTrace.length}<RunActivity entries={stepTrace} />{:else if !isHuman(step.stepId) && step.status !== "running"}<p class="muted">No activity linked to this attempt.</p>{/if}{#if step.input}<details><summary>Handoff / context received by this step</summary><pre>{step.input}</pre></details>{/if}</li>{/each}</ol>{:else}<p class="muted">No steps have started yet.</p>{/if}
	</DetailPanel>
	<DetailPanel title="Run details" id="diagnostics">
		<dl class="task-facts"><div><dt>Task</dt><dd>{#if data.task}<a href={`/tasks/${encodeURIComponent(data.task.id)}`}>{data.task.title}</a>{:else if data.run.taskId}Unavailable task ({data.run.taskId}){:else}No linked task{/if}</dd></div><div><dt>Run ID</dt><dd><code>{data.run.id}</code></dd></div><div><dt>Pipeline</dt><dd>{data.run.pipelineId}</dd></div><div><dt>Started</dt><dd>{new Date(data.run.startedAt).toLocaleString()}</dd></div><div><dt>Ended</dt><dd>{data.run.endedAt ? new Date(data.run.endedAt).toLocaleString() : "Not ended"}</dd></div><div><dt>Duration</dt><dd>{["running", "waitingHuman"].includes(data.run.status) ? `${Math.floor(Math.max(0, clock - Date.parse(data.run.startedAt)) / 1000)} s elapsed` : data.run.durationMs == null ? "Not available" : `${data.run.durationMs} ms`}</dd></div></dl>
		<p class="muted">Run-level token usage and cost: not available (provider telemetry is not collected).</p>
		{#if ["running", "waitingHuman"].includes(data.run.status)}{@const latest = data.run.events.at(-1)}<p class="muted">Last persisted event: {latest ? `${new Date(latest.occurredAt).toLocaleString()} — ${latest.message ?? latest.status}` : "not available"}. This timestamp reports observation only; inactivity does not imply a stalled run.</p>{/if}
		<details><summary>Execution trace ({data.run.trace?.length ?? 0})</summary><p class="muted">{legacyTraceMayContainSensitiveContent ? "This historical trace may contain prompts or transcript content; treat it as sensitive." : "Current traces show bounded model request/response and tool call/result status metadata only; prompts, arguments and result contents are omitted."} Rig session files may still contain sensitive content.</p>{#if data.run.trace?.length}<ol class="run-events">{#each data.run.trace as entry}<li><time datetime={entry.occurredAt}>{new Date(entry.occurredAt).toLocaleString()}</time> <strong>{entry.kind}</strong><details><summary>Show entry</summary><pre>{entry.content}</pre></details></li>{/each}</ol>{:else}<p>No persisted runtime trace is available for this run.</p>{/if}</details>
		<details><summary>Run events ({data.run.events.length})</summary><ol class="run-events">{#each data.run.events as event}<li><time datetime={event.occurredAt}>{new Date(event.occurredAt).toLocaleString()}</time> — {event.message ?? event.status}</li>{/each}</ol></details>
	</DetailPanel>
</div>

<style>
	.overview-label, .action-context > span, .step-result > span {
		color: var(--text-muted);
		font-size: 0.67rem;
		font-weight: 700;
		letter-spacing: 0.08em;
	}
	.action-panel, .failure-summary { margin-bottom: 1.25rem; }
	.failure-summary p { margin: 0.5rem 0; white-space: pre-wrap; overflow-wrap: anywhere; }
	.pipeline-progress { margin: 0 0 1.25rem; padding: 1rem; border: 1px solid var(--border); border-radius: 0.55rem; background: var(--surface-raised); }
	.pipeline-progress ol { display: flex; flex-wrap: wrap; justify-content: space-around; gap: 1rem; padding: 0; margin: 0; list-style: none; }
	.pipeline-progress li { display: grid; justify-items: center; gap: 0.45rem; min-width: 6rem; text-align: center; }
	.progress-name { color: var(--text); font-size: 0.78rem; font-weight: 650; }
	.progress-dot { width: 0.65rem; height: 0.65rem; border: 1px solid var(--text-muted); border-radius: 50%; }
	.progress-dot.visited { border-color: var(--status-success); background: var(--status-success); }
	.progress-dot.current { border-color: var(--status-running); background: var(--status-running); box-shadow: 0 0 0 0.2rem var(--surface-hover); }
	.progress-dot.failed { border-color: var(--status-failed); background: var(--status-failed); }
	.progress-dot.blocked:not(.current) { border-color: var(--status-waiting); background: var(--status-waiting); }
	.progress-status { color: var(--text-muted); font-size: 0.7rem; }
	.action-panel :global(.detail-panel) { border-left: 3px solid var(--status-waiting); }
	.action-intro { color: var(--text-muted); }
	.action-context, .step-result {
		margin: 1rem 0;
		padding: 0.9rem 1rem;
		border: 1px solid var(--border);
		border-radius: 0.5rem;
		background: var(--surface-muted);
	}
	.action-context :global(.markdown), .step-result :global(.markdown) {
		margin: 0.5rem 0 0;
		color: var(--text);
		line-height: 1.55;
		white-space: pre-wrap;
		overflow-wrap: anywhere;
	}
	.action-panel label { display: block; margin: 1rem 0 0.4rem; }
	.action-panel .page-actions { margin-top: 0.85rem; }
	.timeline { gap: 1rem; }
	.timeline li { padding: 1.1rem; background: var(--surface-raised); }
	.step-heading { display: flex; align-items: center; gap: 0.75rem; min-width: 0; }
	.step-number {
		display: grid;
		place-items: center;
		flex: none;
		width: 2rem;
		height: 2rem;
		border: 1px solid var(--border-strong);
		border-radius: 50%;
		color: var(--text-muted);
		font-size: 0.75rem;
		font-weight: 700;
	}
	.step-meta { margin-top: 0.15rem; color: var(--text-muted); font-size: 0.75rem; overflow-wrap: anywhere; }
	.step-route { display: flex; flex-wrap: wrap; gap: 0.5rem; color: var(--text-muted); font-size: 0.75rem; }
	.timeline details { color: var(--text-muted); font-size: 0.8rem; }
	.timeline details summary { cursor: pointer; }
</style>

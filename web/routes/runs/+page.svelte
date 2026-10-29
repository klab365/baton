<script lang="ts">
import Activity from "@lucide/svelte/icons/activity";
import CircleAlert from "@lucide/svelte/icons/circle-alert";
import Clock3 from "@lucide/svelte/icons/clock-3";
import EmptyState from "../../components/EmptyState.svelte";
import PageHeader from "../../components/PageHeader.svelte";
import StatusBadge from "../../components/StatusBadge.svelte";

let { data } = $props();
function statusTone(status: string) {
	if (status === "waitingHuman") return "review";
	if (["failed", "limitExhausted"].includes(status)) return "failed";
	if (status === "succeeded") return "done";
	if (status === "running") return "working";
	return "neutral";
}
const tasksById = $derived(new Map(data.tasks.map((task) => [task.id, task])));
</script>

<svelte:head><title>Runs · Baton</title></svelte:head>
<PageHeader eyebrow="Execution" title="Runs" description="Inspect every pipeline execution and take action where needed." />

{#if data.runs.length}
	<div class="runs-panel">
		{#each data.runs as run (run.id)}
			{@const task = run.taskId ? tasksById.get(run.taskId) : undefined}
			{@const tone = statusTone(run.status)}
			<a class="run-row" href={`/runs/${run.id}`}>
				<span class="run-indicator tone-{tone}"></span>
				<div class="run-primary"><strong>{task?.title ?? "Pipeline run"}</strong><span>{run.pipelineId}</span></div>
				<StatusBadge status={run.status} />
				<div class="run-step">{run.currentStep ?? (run.endedAt ? "Complete" : "Preparing")}</div>
				<time datetime={run.startedAt}>{new Date(run.startedAt).toLocaleString()}</time>
			</a>
		{/each}
	</div>
{:else}
	<EmptyState title="No runs yet" description="Start a task from the task board to create its first pipeline run." />
{/if}

<style>
	.runs-panel { overflow:hidden; border:1px solid #303137; border-radius:.7rem; background:#1b1c20; }
	:global(.status-badge) { justify-self:start; }.run-row { display:grid; grid-template-columns:auto minmax(12rem,1.5fr) minmax(8rem,.8fr) minmax(9rem,1fr) auto; align-items:center; gap:1rem; padding:.9rem 1rem; border-bottom:1px solid #2c2d33; color:#e8e8eb; text-decoration:none; }.run-row:last-child { border-bottom:0; }.run-row:hover { background:#202126; }.run-indicator { width:.5rem; height:.5rem; border-radius:50%; background:#92939b; box-shadow:0 0 6px currentColor; }.run-primary { display:grid; gap:.15rem; min-width:0; }.run-primary strong { overflow:hidden; font-size:.82rem; text-overflow:ellipsis; white-space:nowrap; }.run-primary span,.run-step,time { overflow:hidden; color:#92939b; font-size:.72rem; text-overflow:ellipsis; white-space:nowrap; }.run-status { font-size:.74rem; font-weight:650; }.tone-working { color:#6fa5ff; }.tone-review { color:#f5c442; }.tone-failed { color:#ff7079; }.tone-done { color:#53d695; }.tone-neutral { color:#9b9ca5; } @media(max-width:760px) { .run-row { grid-template-columns:auto minmax(0,1fr) auto; gap:.7rem; }.run-step,.run-row time { display:none; } }
</style>

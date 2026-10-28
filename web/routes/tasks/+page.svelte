<script lang="ts">
import { invalidate, invalidateAll } from "$app/navigation";
import { onMount } from "svelte";
import { dndzone, TRIGGERS } from "svelte-dnd-action";
import {
	ArrowRight,
	CircleCheck,
	CircleDashed,
	CircleDot,
	Clock3,
	Plus,
	RotateCcw,
} from "@lucide/svelte";
import Button from "../../components/Button.svelte";
import PageHeader from "../../components/PageHeader.svelte";
import StatusMessage from "../../components/StatusMessage.svelte";
import type { TaskBoardRun, TaskBoardTask } from "$lib/task-board";

let { data } = $props();
type Task = TaskBoardTask;
type Run = TaskBoardRun;
type Lane = "draft" | "doing" | "approval" | "done";
type BoardItem = { id: string; task: Task; run: Run | undefined; lane: Lane };
let items = $state<BoardItem[]>([]);
let tasks = $state<Task[]>([]);
let runs = $state<Run[]>([]);
let boardLoaded = $state(false);
let loadError = $state("");
let busyTask = $state("");
let draggingTask = $state("");
let error = $state("");
let feedback = $state("");
const latestRuns = $derived.by(() => {
	const byTask = new Map<string, Run[]>();
	for (const run of runs) {
		if (!run.taskId) continue;
		byTask.set(run.taskId, [...(byTask.get(run.taskId) ?? []), run]);
	}
	return byTask;
});
function laneFor(task: Task, run?: Run): Lane {
	if (task.status === "done" || run?.status === "succeeded") return "done";
	if (run?.status === "waitingHuman") return "approval";
	if (task.status === "doing") return "doing";
	return "draft";
}
function statusFor(item: BoardItem) {
	const error = [
		item.run?.setupError,
		...(item.run?.steps.map((step: { error?: string | null }) => step.error) ??
			[]),
	]
		.filter(Boolean)
		.join(" ")
		.toLowerCase();
	if (item.run?.status === "waitingHuman")
		return { label: "Review pending", tone: "review" };
	if (item.run?.status === "failed" || item.run?.status === "limitExhausted")
		return {
			label: error.includes("ci") ? "CI failed" : "Run failed",
			tone: "failed",
		};
	if (item.run?.status === "cancelled")
		return { label: "Cancelled", tone: "neutral" };
	if (item.run?.status === "succeeded" || item.task.status === "done")
		return { label: "Done", tone: "ready" };
	if (item.task.status === "doing" || item.run?.status === "running")
		return { label: "Working", tone: "working" };
	return { label: "Draft", tone: "neutral" };
}
function syncItems() {
	items = tasks.map((task: Task) => {
		const runs = latestRuns.get(task.id) ?? [];
		const run = runs.toSorted((a, b) =>
			b.startedAt.localeCompare(a.startedAt),
		)[0];
		return { id: task.id, task, run, lane: laneFor(task, run) };
	});
}
$effect(() => {
	let current = true;
	boardLoaded = false;
	loadError = "";
	tasks = [];
	runs = [];
	items = [];
	data.tasks.then(
		(loaded) => {
			if (!current) return;
			tasks = loaded;
			boardLoaded = true;
		},
		(cause) => {
			if (!current) return;
			loadError =
				cause instanceof Error ? cause.message : "Could not load tasks.";
			boardLoaded = true;
		},
	);
	data.runs.then(
		(loaded) => {
			if (current) runs = loaded;
		},
		(cause) => {
			if (current)
				loadError ||=
					cause instanceof Error
						? cause.message
						: "Could not load run statuses.";
		},
	);
	return () => {
		current = false;
	};
});
$effect(() => {
	tasks;
	runs;
	if (boardLoaded) syncItems();
});
onMount(() => {
	const refresh = () => {
		if (!document.hidden)
			void Promise.all([invalidate("/api/tasks"), invalidate("/api/runs")]);
	};
	const interval = window.setInterval(refresh, 5000);
	return () => window.clearInterval(interval);
});
function laneItems(lane: Lane) {
	return items.filter((item) => item.lane === lane);
}
function canMove(item: BoardItem, from: Lane, to: Lane) {
	return (
		(from === "draft" && to === "doing") ||
		(from === "doing" && item.run?.status === "succeeded" && to === "done")
	);
}
function dndType(lane: Lane) {
	const item = items.find((candidate) => candidate.id === draggingTask);
	if (!item) return "baton-tasks";
	const from = laneFor(item.task, item.run);
	return lane === from || canMove(item, from, lane)
		? `baton-task-${item.id}`
		: `baton-task-blocked-${lane}`;
}
function consider(
	event: CustomEvent<{ items: BoardItem[]; info: { id: string } }>,
	lane: Lane,
) {
	draggingTask ||= event.detail.info.id;
	const ids = new Set(event.detail.items.map((item) => item.id));
	items = items.map((item) => (ids.has(item.id) ? { ...item, lane } : item));
}
async function finalize(
	event: CustomEvent<{
		items: BoardItem[];
		info: { id: string; trigger: TRIGGERS };
	}>,
	lane: Lane,
) {
	if (event.detail.info.trigger !== TRIGGERS.DROPPED_INTO_ZONE) {
		if (event.detail.info.trigger === TRIGGERS.DROPPED_OUTSIDE_OF_ANY) {
			syncItems();
			draggingTask = "";
		}
		return;
	}
	const item = items.find((candidate) => candidate.id === event.detail.info.id);
	if (!item) return;
	await moveTask(item, lane);
}
async function moveTask(item: BoardItem, lane: Lane) {
	const from = laneFor(item.task, item.run);
	const retrying =
		from === lane && item.run?.status === "failed" && lane === "doing";
	if (busyTask || (from === lane && !retrying)) {
		draggingTask = "";
		return;
	}
	if (!retrying && !canMove(item, from, lane)) {
		syncItems();
		draggingTask = "";
		return;
	}
	busyTask = item.id;
	error = "";
	feedback = "";
	items = items.map((candidate) =>
		candidate.id === item.id ? { ...candidate, lane } : candidate,
	);
	try {
		let response: Response;
		if (lane === "doing") {
			response = await fetch(`/api/tasks/${encodeURIComponent(item.id)}/runs`, {
				method: "POST",
			});
		} else {
			response = await fetch(`/api/tasks/${encodeURIComponent(item.id)}`, {
				method: "PUT",
				headers: { "Content-Type": "application/json" },
				body: JSON.stringify({
					title: item.task.title,
					description: item.task.description,
					pipelineId: item.task.pipelineId,
					status: "done",
				}),
			});
		}
		if (!response.ok) {
			const body = await response.json().catch(() => ({}));
			throw new Error(body.error ?? `Could not move task to ${lane}.`);
		}
		feedback = `Task moved to ${lane === "doing" ? "Doing" : "Done"}.`;
		await invalidateAll();
	} catch (cause) {
		error = cause instanceof Error ? cause.message : "Could not update task.";
		syncItems();
	} finally {
		busyTask = "";
		draggingTask = "";
	}
}
const lanes: {
	id: Lane;
	title: string;
	icon: typeof CircleDashed;
	hint: string;
}[] = [
	{ id: "draft", title: "Draft", icon: CircleDashed, hint: "Ready to start" },
	{ id: "doing", title: "Doing", icon: CircleDot, hint: "Agent runs" },
	{
		id: "approval",
		title: "Needs approval",
		icon: Clock3,
		hint: "Waiting for you",
	},
	{ id: "done", title: "Done", icon: CircleCheck, hint: "Completed" },
];
</script>

<svelte:head><title>Task board · Baton</title></svelte:head>
<PageHeader eyebrow="Workspace" title="Task board" description="Move a task to Doing to start its pipeline. Human decisions show up in Needs approval.">
	{#snippet actions()}<a class="button" href="/tasks/new"><Plus size={16} /> New task</a>{/snippet}
</PageHeader>
{#if error}<StatusMessage tone="error" title="Task update failed">{error}</StatusMessage>{/if}
{#if feedback}<StatusMessage tone="success" title="Task updated">{feedback}</StatusMessage>{/if}
{#if loadError}<StatusMessage tone="error" title="Could not load tasks">{loadError}</StatusMessage>{/if}
{#if !boardLoaded}
	<div class="board board-loading" aria-label="Loading task board" aria-busy="true">
		{#each lanes as lane (lane.id)}
			<section class="board-lane"><header class="lane-header"><div class="lane-heading"><span class="skeleton-dot"></span><h2>{lane.title}</h2><span class="skeleton-count"></span></div><p>{lane.hint}</p></header><div class="lane-cards"><div class="skeleton-card"><span></span><span></span><span></span></div><div class="skeleton-card short"><span></span><span></span></div></div></section>
		{/each}
	</div>
{:else}
<div class="board" aria-label="Task board">
	{#each lanes as lane (lane.id)}
		{@const Icon = lane.icon}
		<section class="board-lane lane-{lane.id}" aria-label={`${lane.title} tasks`}>
			<header class="lane-header">
				<div class="lane-heading"><Icon size={17} aria-hidden="true" /><h2>{lane.title}</h2><span class="lane-count">{laneItems(lane.id).length}</span></div>
				<p>{lane.hint}</p>
			</header>
			<div class="lane-cards-shell">
			<div class="lane-cards" class:dragging={!!draggingTask} use:dndzone={{ items: laneItems(lane.id), type: dndType(lane.id), flipDurationMs: 160, dragDisabled: !!busyTask, dropTargetClasses: ["task-drop-target"], delayTouchStart: true, useCursorForDetection: true }} onconsider={(event) => consider(event, lane.id)} onfinalize={(event) => finalize(event, lane.id)}>
				{#each laneItems(lane.id) as item (item.id)}
					{@const isDragging = draggingTask === item.id}
					{@const status = statusFor(item)}
					<article class="task-card" aria-label={item.task.title} class:card-busy={busyTask === item.id} class:card-dragging={isDragging}>
						<div class="card-topline"><span class="task-id">TASK-{item.id.slice(0, 6).toUpperCase()}</span></div>
						<a class="task-title" href={`/tasks/${item.id}`}>{item.task.title}</a>
						{#if item.task.description}<p class="task-description">{item.task.description}</p>{/if}
						<div class="card-footer"><span class="card-status status-{status.tone}"><span></span>{status.label}</span>
							{#if lane.id === "draft"}<button class="card-action" type="button" disabled={!!busyTask} onclick={() => moveTask(item, "doing")}>{busyTask === item.id ? "Starting…" : "Start task"}<ArrowRight size={14} /></button>
							{:else if lane.id === "approval"}<a class="card-action" href={`/runs/${item.run?.id}`}>Review run<ArrowRight size={14} /></a>
							{:else if lane.id === "doing" && item.run?.status === "failed"}<button class="card-action" type="button" disabled={!!busyTask} onclick={() => moveTask(item, "doing")}><RotateCcw size={14} /> Retry</button>
							{:else if lane.id === "doing" && item.run?.status === "succeeded"}<button class="card-action" type="button" disabled={!!busyTask} onclick={() => moveTask(item, "done")}>Mark done<ArrowRight size={14} /></button>
							{/if}
						</div>
					</article>
				{/each}
			</div>
			{#if !laneItems(lane.id).length}<div class="lane-empty">Drop a task here</div>{/if}
			</div>
		</section>
	{/each}
</div>
{/if}

<style>
	:global(body) { background:#101114; color:#ededf0; }
	.board { display:grid; grid-template-columns:repeat(4,minmax(0,1fr)); min-height:calc(100vh - 12rem); border:1px solid #28292e; border-radius:.8rem; background:#141518; overflow:auto; }
	.board-lane { min-width:0; border-right:1px solid #28292e; }
	.board-lane:last-child { border-right:0; }
	.lane-header { padding:1rem 1rem .75rem; border-bottom:1px solid #28292e; }
	.lane-heading { display:flex; align-items:center; gap:.55rem; }
	.lane-heading h2 { margin:0; font-size:.9rem; font-weight:650; }
	.lane-heading :global(svg) { color:#729cf5; }
	.lane-approval .lane-heading :global(svg) { color:#f5a742; }
	.lane-done .lane-heading :global(svg) { color:#53d695; }
	.lane-count { margin-left:auto; color:#85868f; font-size:.78rem; }
	.lane-header p { margin:.3rem 0 0 1.5rem; color:#777982; font-size:.73rem; }
	.lane-cards-shell { position:relative; }
	.lane-cards { display:flex; flex-direction:column; gap:.65rem; min-height:14rem; padding:.7rem; transition:background .15s,outline-color .15s; }
	:global(.task-drop-target) { outline:2px solid #729cf5; outline-offset:-2px; background:#1c2639; }
	.task-card { padding:.85rem; border:1px solid #303137; border-radius:.65rem; background:#1d1e22; box-shadow:0 2px 6px #0003; cursor:grab; transition:border-color .15s,background .15s,opacity .15s; }
	.card-dragging { border-color:#93b3ff; opacity:.7; }
	.task-card:hover { border-color:#50515a; background:#222329; }
	.card-busy { opacity:.6; }
	.card-topline,.card-footer { display:flex; justify-content:space-between; align-items:center; gap:.5rem; }
	.task-id { color:#80818a; font: .68rem ui-monospace,monospace; }
	.card-status { display:inline-flex; align-items:center; gap:.35rem; color:#aaa; font-size:.7rem; font-weight:600; }
	.card-status > span { width:.4rem; height:.4rem; border-radius:50%; background:currentColor; box-shadow:0 0 7px currentColor; }
	.status-working { color:#6fa5ff; }
	.status-review { color:#f5c442; }
	.status-failed { color:#ff7079; }
	.status-ready { color:#53d695; }
	.status-neutral { color:#9b9ca5; }
	.task-title { display:block; margin:.6rem 0 .3rem; color:#eeeeef; font-size:.88rem; font-weight:650; line-height:1.35; text-decoration:none; }
	.task-title:hover { color:#93b3ff; }
	.task-description { display:-webkit-box; overflow:hidden; line-clamp:3; -webkit-line-clamp:3; -webkit-box-orient:vertical; margin:.35rem 0 .8rem; color:#92939b; font-size:.76rem; line-height:1.45; }
	.card-footer { padding-top:.65rem; border-top:1px solid #303137; }
	.pipeline-tag { max-width:55%; overflow:hidden; color:#85868f; font-size:.68rem; text-overflow:ellipsis; white-space:nowrap; }
	.card-action { display:inline-flex; align-items:center; gap:.3rem; border:0; padding:.25rem 0; background:transparent; color:#8eaeff; font:inherit; font-size:.72rem; cursor:pointer; text-decoration:none; }
	.card-action:hover { color:#c2d2ff; }
	.card-action:disabled { opacity:.45; cursor:wait; }
	.done-mark { display:flex; gap:.3rem; align-items:center; color:#53d695; font-size:.72rem; }
	.lane-empty { position:absolute; inset:.7rem; display:grid; place-items:center; border:1px dashed #303137; border-radius:.55rem; padding:1rem .5rem; color:#62636b; pointer-events:none; text-align:center; font-size:.75rem; }
	.skeleton-dot,.skeleton-count,.skeleton-card span { display:block; background:linear-gradient(90deg,#24252a 25%,#303138 50%,#24252a 75%); background-size:200% 100%; animation:shimmer 1.3s ease-in-out infinite; }
	.skeleton-dot { width:.55rem; height:.55rem; border-radius:50%; }
	.skeleton-count { width:1.2rem; height:.7rem; border-radius:.4rem; }
	.skeleton-card { display:grid; gap:.8rem; padding:.9rem; border:1px solid #303137; border-radius:.65rem; background:#1d1e22; }
	.skeleton-card span { height:.65rem; border-radius:.25rem; }
	.skeleton-card span:first-child { width:42%; }
	.skeleton-card span:nth-child(2) { width:90%; height:1rem; }
	.skeleton-card span:last-child { width:68%; }
	.skeleton-card.short span:last-child { display:none; }
	@keyframes shimmer { to { background-position:-200% 0; } }
	@media(prefers-reduced-motion:reduce) { .skeleton-dot,.skeleton-count,.skeleton-card span { animation:none; } }
	@media(max-width:1050px) { .board { grid-template-columns:repeat(2,minmax(15rem,1fr)); } .board-lane:nth-child(2) { border-right:0; } .board-lane:nth-child(-n+2) { border-bottom:1px solid #28292e; } }
	@media(max-width:620px) { .board { display:flex; flex-direction:column; overflow:visible; } .board-lane { border-right:0; border-bottom:1px solid #28292e; } .board-lane:last-child { border-bottom:0; } .lane-cards { min-height:6rem; } }
</style>

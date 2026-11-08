<script lang="ts">
import Activity from "@lucide/svelte/icons/activity";
import Bot from "@lucide/svelte/icons/bot";
import CheckCircle2 from "@lucide/svelte/icons/check-circle-2";
import KeyRound from "@lucide/svelte/icons/key-round";
import ListTodo from "@lucide/svelte/icons/list-todo";
import Workflow from "@lucide/svelte/icons/workflow";
import PageHeader from "../components/PageHeader.svelte";
import StatusBadge from "../components/StatusBadge.svelte";

type Task = {
	id: string;
	title: string;
	status: "draft" | "doing" | "done";
	updatedAt: string;
};
type Run = {
	id: string;
	taskId?: string | null;
	status: string;
	startedAt: string;
	currentStep?: string | null;
};
type Credential = {
	id: string;
	status: "available" | "missing_secret" | "unavailable";
};
let { data } = $props();
const tasks = $derived(data.tasks as Task[]);
const runs = $derived(data.runs as Run[]);
const credentials = $derived(data.credentials as Credential[]);
const activeRuns = $derived(runs.filter((run) => run.status === "running"));
const approvalRuns = $derived(
	runs.filter((run) => run.status === "waitingHuman"),
);
const failedRuns = $derived(
	runs.filter((run) => ["failed", "limitExhausted"].includes(run.status)),
);
const recentRuns = $derived(
	runs.toSorted((a, b) => b.startedAt.localeCompare(a.startedAt)).slice(0, 5),
);
const availableCredentials = $derived(
	credentials.filter((credential) => credential.status === "available").length,
);
const agentCount = $derived(data.project.config?.agents.length ?? 0);
const pipelineCount = $derived(data.project.config?.pipelines.length ?? 0);
</script>

<svelte:head><title>Overview · Baton</title></svelte:head>

{#if data.project.status === "ready"}
	<PageHeader eyebrow="Workspace" title="Overview" description="Monitor the work moving through your agents and pipelines." />
	<section class="metric-grid" aria-label="Project summary">
		<a class="metric-card metric-credentials" href="/settings"><KeyRound size={18} /><span>Credentials</span><strong>{availableCredentials}<small> / {credentials.length}</small></strong><p>{availableCredentials === credentials.length ? "Ready to use" : "Needs attention"}</p></a>
		<a class="metric-card" href="/agents"><Bot size={18} /><span>Agents</span><strong>{agentCount}</strong><p>Configured roles</p></a>
		<a class="metric-card" href="/pipelines"><Workflow size={18} /><span>Pipelines</span><strong>{pipelineCount}</strong><p>Available workflows</p></a>
		<a class="metric-card" href="/tasks"><ListTodo size={18} /><span>Tasks</span><strong>{tasks.length}</strong><p>{tasks.filter((task) => task.status === "doing").length} in progress</p></a>
	</section>

	<section class="overview-grid">
		<div class="overview-panel attention-panel">
			<header><div><span class="eyebrow">Attention</span><h2>Runs needing action</h2></div><a href="/tasks">Open board</a></header>
			{#if approvalRuns.length || failedRuns.length || activeRuns.length}
				<ul class="attention-list">
					{#each approvalRuns as run}<li><StatusBadge status={run.status} /><a href={`/runs/${run.id}`}><strong>{run.currentStep ?? "Waiting for your decision"}</strong><span>Human action required</span></a></li>{/each}
					{#each failedRuns as run}<li><StatusBadge status={run.status} /><a href={`/runs/${run.id}`}><strong>{run.currentStep ?? "Open run details"}</strong><span>Run requires attention</span></a></li>{/each}
					{#each activeRuns as run}<li><StatusBadge status={run.status} /><a href={`/runs/${run.id}`}><strong>{run.currentStep ?? "Pipeline is running"}</strong><span>Run in progress</span></a></li>{/each}
				</ul>
			{:else}<div class="quiet-state"><CheckCircle2 size={19} /><span>No active or blocked runs.</span></div>{/if}
		</div>
		<div class="overview-panel">
			<header><div><span class="eyebrow">Recent activity</span><h2>Latest runs</h2></div><a href="/tasks">Task board</a></header>
			{#if recentRuns.length}<ul class="activity-list">{#each recentRuns as run}<li><StatusBadge status={run.status} /><a href={`/runs/${run.id}`}><strong>{run.currentStep ?? "Pipeline run"}</strong><span>{new Date(run.startedAt).toLocaleString()}</span></a></li>{/each}</ul>{:else}<div class="quiet-state"><Activity size={19} /><span>Start a task to see activity here.</span></div>{/if}
		</div>
	</section>

	<section class="overview-panel setup-panel">
		<header><div><span class="eyebrow">Get started</span><h2>Project readiness</h2></div></header>
		<div class="setup-steps">
			<a class:complete={availableCredentials > 0} href="/settings"><span>1</span><div><strong>Connect credentials</strong><small>{availableCredentials ? `${availableCredentials} available` : "Add provider access"}</small></div></a>
			<a class:complete={agentCount > 0} href="/agents"><span>2</span><div><strong>Configure agents</strong><small>{agentCount ? `${agentCount} configured` : "Create a reusable role"}</small></div></a>
			<a class:complete={pipelineCount > 0} href="/pipelines"><span>3</span><div><strong>Build a pipeline</strong><small>{pipelineCount ? `${pipelineCount} available` : "Connect your agent steps"}</small></div></a>
			<a class:complete={tasks.length > 0} href="/tasks"><span>4</span><div><strong>Create a task</strong><small>{tasks.length ? `${tasks.length} in the board` : "Send work through a pipeline"}</small></div></a>
		</div>
	</section>
{/if}

<style>
	.metric-grid { display:grid; grid-template-columns:repeat(4,minmax(0,1fr)); gap:.8rem; margin-bottom:1.5rem; }.metric-card,.overview-panel { border:1px solid #303137; border-radius:.7rem; background:#1b1c20; }.metric-card { display:grid; gap:.25rem; padding:1rem; color:#92939b; text-decoration:none; }.metric-card:hover { border-color:#44506b; background:#202126; }.metric-card :global(svg) { color:#9db7ff; }.metric-card span { font-size:.75rem; }.metric-card strong { color:#f0f0f2; font-size:1.55rem; letter-spacing:-.04em; }.metric-card strong small { color:#85868f; font-size:.78rem; font-weight:500; }.metric-credentials strong { font-size:1.2rem; }.metric-credentials strong small { font-size:1.2rem; }.metric-card p { margin:0; font-size:.72rem; }.overview-grid { display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); gap:1rem; margin-bottom:1rem; }.overview-panel { padding:1rem; }.overview-panel header { display:flex; justify-content:space-between; gap:1rem; align-items:start; margin-bottom:.8rem; }.overview-panel h2 { margin:.15rem 0 0; color:#f0f0f2; font-size:1rem; }.overview-panel header a { color:#9db7ff; font-size:.76rem; text-decoration:none; }.attention-list,.activity-list { display:grid; gap:.25rem; padding:0; margin:0; list-style:none; }.attention-list li,.activity-list li { display:flex; align-items:center; gap:.6rem; padding:.65rem .25rem; border-top:1px solid #2c2d33; }.attention-list a,.activity-list a { display:grid; gap:.12rem; min-width:0; color:#e8e8eb; text-decoration:none; }.attention-list strong,.activity-list strong { font-size:.78rem; }.attention-list a span,.activity-list a span { overflow:hidden; color:#92939b; font-size:.72rem; text-overflow:ellipsis; white-space:nowrap; }.quiet-state { display:flex; align-items:center; gap:.55rem; min-height:5rem; color:#92939b; font-size:.78rem; }.quiet-state :global(svg) { color:#53d695; }.setup-panel { margin-top:1rem; }.setup-steps { display:grid; grid-template-columns:repeat(4,minmax(0,1fr)); gap:.65rem; }.setup-steps a { display:flex; gap:.6rem; padding:.75rem; border:1px solid #303137; border-radius:.55rem; color:#e8e8eb; text-decoration:none; }.setup-steps a:hover { background:#202126; }.setup-steps a > span { display:grid; place-items:center; flex:none; width:1.35rem; height:1.35rem; border:1px solid #4a4b54; border-radius:50%; color:#92939b; font-size:.68rem; }.setup-steps a.complete > span { border-color:#3d916a; background:#1c342b; color:#70e3a8; }.setup-steps div { display:grid; gap:.12rem; min-width:0; }.setup-steps strong { font-size:.77rem; }.setup-steps small { color:#92939b; font-size:.69rem; line-height:1.3; } @media(max-width:900px) { .metric-grid,.setup-steps { grid-template-columns:repeat(2,minmax(0,1fr)); } } @media(max-width:620px) { .metric-grid,.overview-grid,.setup-steps { grid-template-columns:1fr; } }
</style>

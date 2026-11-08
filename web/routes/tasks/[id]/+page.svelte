<script lang="ts">
import { goto, invalidate, invalidateAll } from "$app/navigation";
import { onMount } from "svelte";
import Button from "../../../components/Button.svelte";
import ConfirmationDialog from "../../../components/ConfirmationDialog.svelte";
import DetailPanel from "../../../components/DetailPanel.svelte";
import PageHeader from "../../../components/PageHeader.svelte";
import StatusMessage from "../../../components/StatusMessage.svelte";
import StatusBadge from "../../../components/StatusBadge.svelte";
import TaskForm from "../../../components/TaskForm.svelte";
import Field from "../../../components/Field.svelte";
import MultilineInput from "../../../components/MultilineInput.svelte";
let { data } = $props();
let confirm = $state(false);
let loading = $state(false);
let error = $state("");
let followUp = $state("");
let savedFollowUp = $state("");
onMount(() => {
	const refresh = () => {
		if (!document.hidden) {
			const taskUrl = `/api/tasks/${encodeURIComponent(data.task.id)}`;
			void Promise.all([invalidate(taskUrl), invalidate(`${taskUrl}/runs`)]);
		}
	};
	const interval = window.setInterval(refresh, 2000);
	document.addEventListener("visibilitychange", refresh);
	return () => {
		window.clearInterval(interval);
		document.removeEventListener("visibilitychange", refresh);
	};
});
let completed = $derived(
	data.runs.length
		? [...data.runs].sort((a, b) => b.startedAt.localeCompare(a.startedAt))[0]
				.status === "succeeded"
		: data.task.status === "done",
);
let selectedPipeline = $derived(
	data.project.config?.pipelines.find(
		(pipeline: { id: string }) => pipeline.id === data.task.pipelineId,
	),
);
async function start() {
	loading = true;
	error = "";
	try {
		const response = await fetch(`/api/tasks/${data.task.id}/runs`, {
			method: "POST",
		});
		if (!response.ok)
			throw new Error((await response.json()).error ?? "Could not start task.");
		await invalidateAll();
	} catch (e) {
		error = e instanceof Error ? e.message : "Could not start task.";
	} finally {
		loading = false;
	}
}
async function reopen(event: SubmitEvent) {
	event.preventDefault();
	if (!followUp.trim()) return;
	loading = true;
	error = "";
	try {
		if (savedFollowUp !== followUp) {
			const response = await fetch(
				`/api/tasks/${encodeURIComponent(data.task.id)}`,
				{
					method: "PUT",
					headers: { "Content-Type": "application/json" },
					body: JSON.stringify({
						title: data.task.title,
						description: `${data.task.description}\n\nFollow-up work:\n${followUp.trim()}`,
						pipelineId: data.task.pipelineId,
						status: "done",
					}),
				},
			);
			if (!response.ok)
				throw new Error(
					(await response.json()).error ?? "Could not save follow-up work.",
				);
			savedFollowUp = followUp;
		}
		const response = await fetch(
			`/api/tasks/${encodeURIComponent(data.task.id)}/runs`,
			{ method: "POST" },
		);
		if (!response.ok)
			throw new Error((await response.json()).error ?? "Could not start task.");
		followUp = "";
		savedFollowUp = "";
		await invalidateAll();
	} catch (e) {
		error = e instanceof Error ? e.message : "Could not reopen task.";
	} finally {
		loading = false;
	}
}
async function remove() {
	loading = true;
	try {
		const response = await fetch(`/api/tasks/${data.task.id}`, {
			method: "DELETE",
		});
		if (!response.ok)
			throw new Error(
				(await response.json()).error ?? "Could not delete task.",
			);
		await goto("/tasks", { invalidateAll: true });
	} catch (e) {
		error = e instanceof Error ? e.message : "Could not delete task.";
	} finally {
		loading = false;
	}
}
</script>

<svelte:head><title>Task · Baton</title></svelte:head>
<PageHeader eyebrow="Task" title={data.task.title} description="Review or update this task.">
	{#snippet actions()}{#if !completed}<Button {loading} onclick={start}>Start task</Button>{/if}<Button variant="danger" onclick={() => (confirm = true)}>Delete task</Button>{/snippet}
</PageHeader>
{#if error}<StatusMessage tone="error">{error}</StatusMessage>{/if}
<div class="task-detail-layout">
	<DetailPanel title="Task state">
		<dl class="task-facts">
			<div><dt>Status</dt><dd><StatusBadge status={data.task.status} /></dd></div>
			<div><dt>Pipeline</dt><dd>{selectedPipeline?.id ?? `Unavailable pipeline (${data.task.pipelineId})`}</dd></div>
			<div><dt>Created</dt><dd>{new Date(data.task.createdAt).toLocaleString()}</dd></div>
			<div><dt>Last updated</dt><dd>{new Date(data.task.updatedAt).toLocaleString()}</dd></div>
		</dl>
	</DetailPanel>
	<DetailPanel title="Run history">
		{#if data.runs.length}
			<ul class="run-list">
				{#each data.runs as run}
					<li><a href={`/runs/${run.id}`}>{run.id}</a><StatusBadge status={run.status} /><time datetime={run.startedAt}>{new Date(run.startedAt).toLocaleString()}</time></li>
				{/each}
			</ul>
		{:else}<p class="muted">This task has not been started yet.</p>{/if}
	</DetailPanel>
	{#if completed}
		<div id="reopen"><DetailPanel title="Reopen task">
			<form onsubmit={reopen}>
				<Field label="Follow-up work" for="follow-up" hint="Describe the missing work to include in the next run." required>
					{#snippet children({ describedBy, invalid })}<MultilineInput id="follow-up" name="follow-up" bind:value={followUp} rows={4} {describedBy} {invalid} />{/snippet}
				</Field>
				<Button type="submit" {loading} disabled={!followUp.trim()}>Reopen &amp; rerun</Button>
			</form>
		</DetailPanel></div>
	{/if}
	<div id="edit"><DetailPanel title="Edit task">
		{#key data.task.id}<TaskForm task={data.task} pipelines={data.project.config?.pipelines ?? []} />{/key}
	</DetailPanel></div>
</div>
<ConfirmationDialog bind:open={confirm} title="Delete task and worktree?" {loading} onconfirm={remove}>This permanently deletes the task, its worktree, uncommitted changes, and Rig session data.</ConfirmationDialog>

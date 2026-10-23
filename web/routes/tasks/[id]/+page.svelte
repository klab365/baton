<script lang="ts">
import { goto } from "$app/navigation";
import Button from "../../../components/Button.svelte";
import ConfirmationDialog from "../../../components/ConfirmationDialog.svelte";
import DetailPanel from "../../../components/DetailPanel.svelte";
import PageHeader from "../../../components/PageHeader.svelte";
import StatusMessage from "../../../components/StatusMessage.svelte";
import TaskForm from "../../../components/TaskForm.svelte";
let { data } = $props();
let confirm = $state(false);
let loading = $state(false);
let error = $state("");
let selectedPipeline = $derived(
	data.task.configSnapshot.pipelines.find(
		(pipeline: { id: string }) => pipeline.id === data.task.pipelineId,
	),
);
async function remove() {
	loading = true;
	try {
		const response = await fetch(`/api/tasks/${data.task.id}`, {
			method: "DELETE",
		});
		if (!response.ok) throw new Error("Could not delete task.");
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
	{#snippet actions()}<Button variant="danger" onclick={() => (confirm = true)}>Delete task</Button>{/snippet}
</PageHeader>
{#if error}<StatusMessage tone="error">{error}</StatusMessage>{/if}
<div class="task-detail-layout">
	<DetailPanel title="Task state">
		<dl class="task-facts">
			<div><dt>Status</dt><dd><span class="task-status">{data.task.status}</span></dd></div>
			<div><dt>Pipeline</dt><dd>{selectedPipeline?.name ?? "Unavailable pipeline"}<span class="task-pipeline-id">{data.task.pipelineId}</span></dd></div>
			<div><dt>Created</dt><dd>{new Date(data.task.createdAt).toLocaleString()}</dd></div>
			<div><dt>Last updated</dt><dd>{new Date(data.task.updatedAt).toLocaleString()}</dd></div>
		</dl>
	</DetailPanel>
	<div id="edit"><DetailPanel title="Edit task">
		{#key data.task.id}<TaskForm task={data.task} pipelines={data.project.config?.pipelines ?? []} />{/key}
	</DetailPanel></div>
</div>
<ConfirmationDialog bind:open={confirm} title="Delete task?" {loading} onconfirm={remove}>This permanently deletes the task.</ConfirmationDialog>

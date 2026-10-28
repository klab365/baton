<script lang="ts">
import { goto } from "$app/navigation";
import EmptyState from "../../components/EmptyState.svelte";
import PageHeader from "../../components/PageHeader.svelte";
let { data } = $props();
</script>

<svelte:head><title>Tasks · Baton</title></svelte:head>
<PageHeader title="Tasks" description="Manage task prompts and pipeline selections.">
	{#snippet actions()}<a class="button" href="/tasks/new">Create task</a>{/snippet}
</PageHeader>
{#if data.tasks.length}
	<div class="task-table-wrap">
		<table class="task-table">
			<caption class="visually-hidden">Tasks</caption>
			<thead>
				<tr><th scope="col">Task</th><th scope="col">Pipeline</th><th scope="col">Status</th><th scope="col">Updated</th></tr>
			</thead>
			<tbody>
				{#each data.tasks as task}
					<tr ondblclick={() => goto(`/tasks/${task.id}#edit`)}>
						<th scope="row"><a class="task-table-title" href="/tasks/{task.id}#edit">{task.title}</a></th>
						<td data-label="Pipeline"><span class="task-pipeline-name">{data.project.config?.pipelines.find((pipeline: { id: string }) => pipeline.id === task.pipelineId)?.name ?? "Unavailable pipeline"}</span><span class="task-pipeline-id">{task.pipelineId}</span></td>
						<td data-label="Status"><span class="task-status">{task.status}</span></td>
						<td data-label="Updated"><time datetime={task.updatedAt}>{new Date(task.updatedAt).toLocaleDateString()}</time></td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>
{:else}
	<EmptyState title="No tasks yet" description="Create a task using an existing configured pipeline.">
		{#snippet children()}<a class="button" href="/tasks/new">Create task</a>{/snippet}
	</EmptyState>
{/if}

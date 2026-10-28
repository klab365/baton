<script lang="ts">
import { goto, invalidateAll } from "$app/navigation";
import { untrack } from "svelte";
import Button from "./Button.svelte";
import Field from "./Field.svelte";
import MultilineInput from "./MultilineInput.svelte";
import StatusMessage from "./StatusMessage.svelte";
import ResourceSelect from "./ResourceSelect.svelte";
import type { Pipeline } from "$lib/generated/Pipeline";
let {
	pipelines,
	task,
}: {
	pipelines: Pick<Pipeline, "id" | "name">[];
	task?: {
		id: string;
		title: string;
		description: string;
		acceptanceCriteria: string;
		pipelineId: string;
		status: "draft" | "doing" | "done";
	};
} = $props();
let title = $state(untrack(() => task?.title ?? ""));
let description = $state(untrack(() => task?.description ?? ""));
let acceptanceCriteria = $state(untrack(() => task?.acceptanceCriteria ?? ""));
let pipelineId = $state(untrack(() => task?.pipelineId ?? ""));
let status = $state(untrack(() => task?.status ?? "draft"));
let error = $state("");
let loading = $state(false);
async function save(event: SubmitEvent) {
	event.preventDefault();
	loading = true;
	error = "";
	try {
		const response = await fetch(
			task ? `/api/tasks/${task.id}` : "/api/tasks",
			{
				method: task ? "PUT" : "POST",
				headers: { "Content-Type": "application/json" },
				body: JSON.stringify({
					title,
					description,
					acceptanceCriteria,
					pipelineId,
					status,
				}),
			},
		);
		const text = await response.text();
		let message = text;
		try {
			message = JSON.parse(text).error ?? text;
		} catch {
			// Axum request validation can return a plain-text error response.
		}
		if (!response.ok) throw new Error(message || "Could not save task.");
		await invalidateAll();
		await goto("/tasks");
	} catch (e) {
		error = e instanceof Error ? e.message : "Could not save task.";
	} finally {
		loading = false;
	}
}
</script>

<form class="task-form" onsubmit={save}>
	<div class="task-form-body">
		{#if error}<StatusMessage tone="error">{error}</StatusMessage>{/if}
		<Field label="Title" for="title" hint="A short name for this task." required>
			{#snippet children({ describedBy, invalid })}<input id="title" name="title" bind:value={title} placeholder="e.g. Add user authentication" aria-describedby={describedBy} aria-invalid={invalid} required />{/snippet}
		</Field>
		<Field label="Description" for="description" hint="Describe the task and include any relevant context." required>
			{#snippet children({ describedBy, invalid })}<MultilineInput id="description" name="description" bind:value={description} rows={6} placeholder="What needs to be done? Include background or constraints…" {describedBy} {invalid} />{/snippet}
		</Field>
		<Field label="Acceptance criteria" for="acceptanceCriteria" hint="How will you know this task is complete?">
			{#snippet children({ describedBy, invalid })}<MultilineInput id="acceptanceCriteria" name="acceptanceCriteria" bind:value={acceptanceCriteria} rows={4} placeholder="List the outcomes or conditions that define completion…" {describedBy} {invalid} />{/snippet}
		</Field>
		<Field label="Pipeline" for="pipeline" hint="The selected pipeline is captured with this task." required>
			{#snippet children({ describedBy, invalid })}<ResourceSelect id="pipeline" bind:value={pipelineId} items={pipelines} placeholder="Select a pipeline" kind="pipeline" {describedBy} {invalid} name="pipeline" required />{/snippet}
		</Field>
		{#if !pipelines.length}<StatusMessage tone="warning" title="No pipelines available">Add a pipeline to <code>.baton/config.json</code> before creating a task.</StatusMessage>{/if}
	</div>
	<footer class="task-form-actions"><Button type="submit" {loading} disabled={!title.trim() || !description.trim() || !pipelines.some((p) => p.id === pipelineId)}>{task ? "Save changes" : "Create task"}</Button></footer>
</form>

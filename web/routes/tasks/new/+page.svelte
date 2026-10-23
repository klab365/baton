<script lang="ts">
import PageHeader from "../../../components/PageHeader.svelte";
import TaskForm from "../../../components/TaskForm.svelte";
let { data } = $props();
</script>
<svelte:head><title>Create task · Baton</title></svelte:head>
<PageHeader title="Create task" description="Select an existing pipeline. Saving does not start a run." />
<TaskForm pipelines={data.project.config?.pipelines ?? []} />

<script lang="ts">
import PageHeader from "../components/PageHeader.svelte";
import ResourceList from "../components/ResourceList.svelte";
let { data } = $props();
</script>

<svelte:head><title>Overview · Baton</title></svelte:head>

{#if data.project.status === "ready"}
	<PageHeader eyebrow="Project overview" title="Configure your workflow" description="Build a clear path from specialized agents to repeatable pipelines and the tasks they process." />
	<section aria-labelledby="configuration-flow">
		<h2 id="configuration-flow">Configuration flow</h2>
		<ResourceList label="Configuration flow">
			<li><a href="/agents"><strong>1. Agents</strong><span>Define the roles and capabilities available to your workflows.</span></a></li>
			<li><a href="/pipelines"><strong>2. Pipelines</strong><span>Arrange agent steps, including execution settings, into a workflow.</span></a></li>
			<li><a href="/tasks"><strong>3. Tasks</strong><span>Send work through a configured pipeline.</span></a></li>
		</ResourceList>
	</section>
{/if}

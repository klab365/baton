<script lang="ts">
let { data } = $props();
</script>

<svelte:head><title>Overview · Baton</title></svelte:head>

{#if data.project.status === "ready"}
	<p class="eyebrow">Project overview</p>
	<h1>Welcome to Baton</h1>
	<p class="lead">Baton connects specialized agents into transparent workflows.</p>
	<section class="cards" aria-label="Next steps">
		<article><h2>Tasks</h2><p>Create tasks from prompts or import them from connected systems.</p><a href="/tasks">View tasks</a></article>
		<article><h2>Pipelines</h2><p>Define workflow steps, handoffs, and reviews.</p><a href="/pipelines">View pipelines</a></article>
		<article><h2>Agents</h2><p>Configure agent roles, models, skills, and tools.</p><a href="/agents">View agents</a></article>
	</section>
{/if}
